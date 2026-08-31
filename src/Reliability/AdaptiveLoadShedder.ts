/**
 * AdaptiveLoadShedder
 *
 * Admission control that protects critical execution paths from saturating
 * under load. Decisions are made from a combination of:
 *   - queue depth (pending / in-flight work)
 *   - dependency latency (EWMA-smoothed RPC / DB / worker latency)
 *   - error rate (EWMA-smoothed)
 *   - execution criticality (separate budgets per traffic class)
 *
 * Capacity is preserved for confirmations / recovery work by giving those
 * classes reserved slots that are admitted even while other classes shed.
 * Hysteresis (separate enter / release thresholds) prevents rapid flapping
 * between nominal and shedding modes.
 *
 * This module is intentionally dependency-free (no DB / redis / logger) so it
 * can be unit-tested in isolation and used from any hot path.
 */

export enum TrafficClass {
  READONLY = "readonly",
  EXECUTION = "execution",
  RECOVERY = "recovery",
}

export enum LoadMode {
  NOMINAL = "nominal",
  SHEDDING = "shedding",
}

export enum RejectReason {
  QUEUE_DEPTH = "queue_depth_exceeded",
  LATENCY = "dependency_latency_exceeded",
  ERROR_RATE = "error_rate_exceeded",
  RESERVED_CAPACITY = "new_execution_capacity_reserved",
}

export type AdmissionDecision =
  | { allowed: true; klass: TrafficClass; mode: LoadMode }
  | {
      allowed: false;
      klass: TrafficClass;
      mode: LoadMode;
      reason: RejectReason;
      retryAfterMs: number;
    };

export interface ClassBudget {
  /** Max concurrent in-flight operations admitted for this class. */
  maxInFlight: number;
  /** Max pending queue depth tolerated before this class sheds. */
  maxQueueDepth: number;
  /** Slots always preserved for this class regardless of load. */
  reservedSlots: number;
}

export interface LoadSheddingConfig {
  budget: Record<TrafficClass, ClassBudget>;
  /** Error rate (0..1) above which the controller enters shedding. */
  errorRateEnterThreshold: number;
  /** Error rate (0..1) below which the controller releases shedding. */
  errorRateReleaseThreshold: number;
  /** Dependency latency (ms) above which the controller enters shedding. */
  latencyEnterThresholdMs: number;
  /** Dependency latency (ms) below which the controller releases shedding. */
  latencyReleaseThresholdMs: number;
  /** EWMA smoothing factor (0..1); larger = faster reaction. */
  ewmaAlpha: number;
  /** Baseline retry delay (ms) returned as safe retry guidance. */
  baseRetryDelayMs: number;
}

const DEFAULT_CONFIG: LoadSheddingConfig = {
  budget: {
    [TrafficClass.READONLY]: { maxInFlight: 200, maxQueueDepth: 500, reservedSlots: 40 },
    [TrafficClass.EXECUTION]: { maxInFlight: 100, maxQueueDepth: 250, reservedSlots: 10 },
    [TrafficClass.RECOVERY]: { maxInFlight: 60, maxQueueDepth: 400, reservedSlots: 30 },
  },
  errorRateEnterThreshold: 0.5,
  errorRateReleaseThreshold: 0.2,
  latencyEnterThresholdMs: 150,
  latencyReleaseThresholdMs: 60,
  ewmaAlpha: 0.3,
  baseRetryDelayMs: 250,
};

export class AdaptiveLoadShedder {
  private readonly config: LoadSheddingConfig;

  // EWMA metric state
  private latencyMs = 0;
  private errorRate = 0;

  // Per-class in-flight / queue accounting
  private readonly inFlight = new Map<TrafficClass, number>();
  private readonly queueDepth = new Map<TrafficClass, number>();

  private mode: LoadMode = LoadMode.NOMINAL;

  private operationalWindow = 0;
  private errorWindow = 0;
  private readonly windowSize: number;

  constructor(config: Partial<LoadSheddingConfig> = {}, windowSize = 100) {
    this.config = {
      ...DEFAULT_CONFIG,
      ...config,
      budget: { ...DEFAULT_CONFIG.budget, ...(config.budget ?? {}) },
    };
    this.windowSize = windowSize;
  }

  // ── Metric inputs ──────────────────────────────────────────────────────────

  /** Record a dependency call latency (ms) into the EWMA. */
  observeDependencyLatency(ms: number): void {
    const a = this.config.ewmaAlpha;
    this.latencyMs = this.latencyMs === 0 ? ms : a * ms + (1 - a) * this.latencyMs;
  }

  /** Record a successful dependency call. */
  observeSuccess(): void {
    this.flushErrorWindow();
  }

  /** Record a failed dependency call and recompute the smoothed error rate. */
  observeError(): void {
    this.errorWindow += 1;
    this.flushErrorWindow();
  }

  /** Fold the current sampling window into the smoothed error rate. */
  private flushErrorWindow(): void {
    this.operationalWindow += 1;
    if (this.operationalWindow >= this.windowSize) {
      this.errorRate =
        this.errorRate === 0
          ? this.errorWindow / this.windowSize
          : this.config.ewmaAlpha * (this.errorWindow / this.windowSize) +
            (1 - this.config.ewmaAlpha) * this.errorRate;
      this.operationalWindow = 0;
      this.errorWindow = 0;
    }
  }

  /** Set the current pending-queue depth for a traffic class. */
  setQueueDepth(klass: TrafficClass, depth: number): void {
    this.queueDepth.set(klass, Math.max(0, depth));
  }

  // ── Load mode (hysteresis) ─────────────────────────────────────────────────

  getMode(): LoadMode {
    return this.mode;
  }

  getMetrics(): { latencyMs: number; errorRate: number; mode: LoadMode } {
    return { latencyMs: this.latencyMs, errorRate: this.errorRate, mode: this.mode };
  }

  private refreshMode(...targets: TrafficClass[]): void {
    const c = this.config;
    const classes = targets.length ? targets : (Object.values(TrafficClass) as TrafficClass[]);
    let shed = false;
    let release = true;

    for (const klass of classes) {
      const budget = c.budget[klass];
      const depth = this.queueDepth.get(klass) ?? 0;
      const inFlight = this.inFlight.get(klass) ?? 0;

      const overloaded =
        depth > budget.maxQueueDepth ||
        inFlight >= budget.maxInFlight ||
        this.errorRate >= c.errorRateEnterThreshold ||
        this.latencyMs >= c.latencyEnterThresholdMs;

      const recovered =
        depth <= budget.maxQueueDepth * 0.7 &&
        this.errorRate < c.errorRateReleaseThreshold &&
        this.latencyMs < c.latencyReleaseThresholdMs;

      shed = shed || overloaded;
      release = release && recovered;
    }

    if (this.mode === LoadMode.NOMINAL && shed) {
      this.mode = LoadMode.SHEDDING;
    } else if (this.mode === LoadMode.SHEDDING && release) {
      this.mode = LoadMode.NOMINAL;
    }
  }

  /**
   * Ask for admission of a unit of work of a given traffic class.
   *
   * Returns an `AdmissionDecision`. If rejected, the decision carries safe
   * retry guidance (`retryAfterMs`) so callers can back off without thundering.
   */
  admit(klass: TrafficClass, queueDepth = 0): AdmissionDecision {
    if (queueDepth > 0) this.setQueueDepth(klass, queueDepth);
    this.refreshMode(klass);

    const budget = this.config.budget[klass];
    const inFlight = this.inFlight.get(klass) ?? 0;

    // Reserved slots are always honored for confirmations / recovery work so
    // capacity is preserved for the most critical traffic even under load.
    if (inFlight < budget.reservedSlots && klass !== TrafficClass.EXECUTION) {
      this.acquire(klass);
      return { allowed: true, klass, mode: this.mode };
    }

    if (this.mode === LoadMode.SHEDDING) {
      const reason = this.pickReason(klass);
      return {
        allowed: false,
        klass,
        mode: this.mode,
        reason,
        retryAfterMs: this.retryAfter(reason),
      };
    }

    if (inFlight >= budget.maxInFlight) {
      return {
        allowed: false,
        klass,
        mode: this.mode,
        reason: RejectReason.RESERVED_CAPACITY,
        retryAfterMs: this.retryAfter(RejectReason.RESERVED_CAPACITY),
      };
    }

    this.acquire(klass);
    return { allowed: true, klass, mode: this.mode };
  }

  /** Release a previously-acquired slot. */
  release(klass: TrafficClass): void {
    const current = this.inFlight.get(klass) ?? 0;
    if (current > 0) {
      this.inFlight.set(klass, current - 1);
    }
  }

  /** Reset all runtime state. Primarily useful for tests. */
  reset(): void {
    this.latencyMs = 0;
    this.errorRate = 0;
    this.mode = LoadMode.NOMINAL;
    this.operationalWindow = 0;
    this.errorWindow = 0;
    this.inFlight.clear();
    this.queueDepth.clear();
  }

  // ── Internals ──────────────────────────────────────────────────────────────

  private acquire(klass: TrafficClass): void {
    this.inFlight.set(klass, (this.inFlight.get(klass) ?? 0) + 1);
  }

  private pickReason(klass: TrafficClass): RejectReason {
    const budget = this.config.budget[klass];
    const depth = this.queueDepth.get(klass) ?? 0;
    if (depth > budget.maxQueueDepth) return RejectReason.QUEUE_DEPTH;
    if (this.latencyMs >= this.config.latencyEnterThresholdMs) return RejectReason.LATENCY;
    if (this.errorRate >= this.config.errorRateEnterThreshold) return RejectReason.ERROR_RATE;
    return RejectReason.RESERVED_CAPACITY;
  }

  private retryAfter(reason: RejectReason): number {
    // Provide monotonic, jitter-free safe backoff guidance.
    const base = this.config.baseRetryDelayMs;
    switch (reason) {
      case RejectReason.QUEUE_DEPTH:
        return base * 2;
      case RejectReason.LATENCY:
        return base * 3;
      case RejectReason.ERROR_RATE:
        return base * 4;
      default:
        return base;
    }
  }
}

export const loadShedder = new AdaptiveLoadShedder();
