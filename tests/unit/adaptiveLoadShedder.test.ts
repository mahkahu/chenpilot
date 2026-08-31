import { describe, it, expect } from "@jest/globals";
import {
  AdaptiveLoadShedder,
  TrafficClass,
  LoadMode,
  RejectReason,
} from "../../src/Reliability/AdaptiveLoadShedder";

describe("AdaptiveLoadShedder", () => {
  describe("nominal operation", () => {
    it("admits new execution work while metrics are healthy", () => {
      const shedder = new AdaptiveLoadShedder();
      const decision = shedder.admit(TrafficClass.EXECUTION);
      expect(decision.allowed).toBe(true);
      if (decision.allowed) {
        expect(decision.mode).toBe(LoadMode.NOMINAL);
        expect(decision.klass).toBe(TrafficClass.EXECUTION);
      }
    });

    it("honors maxInFlight concurrency for a traffic class", () => {
      const shedder = new AdaptiveLoadShedder({
        budget: {
          [TrafficClass.EXECUTION]: { maxInFlight: 2, maxQueueDepth: 10, reservedSlots: 1 },
          [TrafficClass.READONLY]: { maxInFlight: 5, maxQueueDepth: 10, reservedSlots: 1 },
          [TrafficClass.RECOVERY]: { maxInFlight: 5, maxQueueDepth: 10, reservedSlots: 1 },
        },
      });
      const first = shedder.admit(TrafficClass.EXECUTION);
      const second = shedder.admit(TrafficClass.EXECUTION);
      const third = shedder.admit(TrafficClass.EXECUTION);

      expect(first.allowed).toBe(true);
      expect(second.allowed).toBe(true);
      expect(third.allowed).toBe(false);

      // Releasing a slot frees capacity again.
      shedder.release(TrafficClass.EXECUTION);
      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(true);
    });
  });

  describe("hysteresis", () => {
    it("enters shedding when latency exceeds the enter threshold", () => {
      const shedder = new AdaptiveLoadShedder({
        latencyEnterThresholdMs: 150,
        latencyReleaseThresholdMs: 60,
        ewmaAlpha: 1,
      });
      shedder.observeDependencyLatency(200);

      expect(shedder.getMode()).toBe(LoadMode.NOMINAL);
      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(false);
      expect(shedder.getMode()).toBe(LoadMode.SHEDDING);
    });

    it("stays in shedding until latency drops below the release threshold", () => {
      const shedder = new AdaptiveLoadShedder({
        latencyEnterThresholdMs: 150,
        latencyReleaseThresholdMs: 60,
        ewmaAlpha: 1,
      });
      shedder.observeDependencyLatency(200);
      shedder.admit(TrafficClass.EXECUTION);
      expect(shedder.getMode()).toBe(LoadMode.SHEDDING);

      // Slight recovery but still above the release threshold → keep shedding.
      shedder.observeDependencyLatency(100);
      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(false);
      expect(shedder.getMode()).toBe(LoadMode.SHEDDING);

      // Full recovery below release threshold → back to nominal and admit.
      shedder.observeDependencyLatency(50);
      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(true);
      expect(shedder.getMode()).toBe(LoadMode.NOMINAL);
    });

    it("sheds on high error rate and recovers once below the release threshold", () => {
      const shedder = new AdaptiveLoadShedder({
        errorRateEnterThreshold: 0.5,
        errorRateReleaseThreshold: 0.2,
        ewmaAlpha: 1,
      });
      // Force a near-100% error window.
      for (let i = 0; i < 100; i++) shedder.observeError();

      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(false);
      expect(shedder.getMode()).toBe(LoadMode.SHEDDING);

      // Subsequent successes pull the smoothed rate below release threshold.
      for (let i = 0; i < 400; i++) shedder.observeSuccess();
      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(true);
      expect(shedder.getMode()).toBe(LoadMode.NOMINAL);
    });

    it("sheds when the pending queue depth exceeds the class budget", () => {
      const shedder = new AdaptiveLoadShedder({
        budget: {
          [TrafficClass.EXECUTION]: { maxInFlight: 10, maxQueueDepth: 5, reservedSlots: 1 },
          [TrafficClass.READONLY]: { maxInFlight: 10, maxQueueDepth: 5, reservedSlots: 1 },
          [TrafficClass.RECOVERY]: { maxInFlight: 10, maxQueueDepth: 5, reservedSlots: 1 },
        },
        ewmaAlpha: 1,
      });
      expect(shedder.admit(TrafficClass.EXECUTION, 6).allowed).toBe(false);
      expect(shedder.admit(TrafficClass.EXECUTION, 2).allowed).toBe(true);
    });
  });

  describe("reserved capacity", () => {
    it("always admits reserved critical work (recovery/readonly) during overload", () => {
      const shedder = new AdaptiveLoadShedder({
        latencyEnterThresholdMs: 50,
        latencyReleaseThresholdMs: 10,
        ewmaAlpha: 1,
        budget: {
          [TrafficClass.EXECUTION]: { maxInFlight: 1, maxQueueDepth: 1, reservedSlots: 1 },
          [TrafficClass.READONLY]: { maxInFlight: 100, maxQueueDepth: 100, reservedSlots: 5 },
          [TrafficClass.RECOVERY]: { maxInFlight: 100, maxQueueDepth: 100, reservedSlots: 5 },
        },
      });
      shedder.observeDependencyLatency(500);
      expect(shedder.admit(TrafficClass.EXECUTION).allowed).toBe(false);

      // Recovery retains its reserved slots even while shedding.
      expect(shedder.admit(TrafficClass.RECOVERY).allowed).toBe(true);
      expect(shedder.admit(TrafficClass.READONLY).allowed).toBe(true);
    });
  });

  describe("rejection guidance", () => {
    it("reports a reject reason and safe retry delay when shedding", () => {
      const shedder = new AdaptiveLoadShedder({
        latencyEnterThresholdMs: 50,
        latencyReleaseThresholdMs: 10,
        ewmaAlpha: 1,
      });
      shedder.observeDependencyLatency(500);
      const decision = shedder.admit(TrafficClass.EXECUTION);
      expect(decision.allowed).toBe(false);
      if (!decision.allowed) {
        expect(decision.mode).toBe(LoadMode.SHEDDING);
        expect(decision.retryAfterMs).toBeGreaterThan(0);
      }
    });

    it("prioritizes queue-depth rejection reason over latency", () => {
      const shedder = new AdaptiveLoadShedder({
        budget: {
          [TrafficClass.EXECUTION]: { maxInFlight: 10, maxQueueDepth: 3, reservedSlots: 1 },
          [TrafficClass.READONLY]: { maxInFlight: 10, maxQueueDepth: 3, reservedSlots: 1 },
          [TrafficClass.RECOVERY]: { maxInFlight: 10, maxQueueDepth: 3, reservedSlots: 1 },
        },
        latencyEnterThresholdMs: 50,
        latencyReleaseThresholdMs: 10,
        ewmaAlpha: 1,
      });
      shedder.observeDependencyLatency(500);
      const decision = shedder.admit(TrafficClass.EXECUTION, 10);
      expect(decision.allowed).toBe(false);
      if (!decision.allowed) {
        expect(decision.reason).toBe(RejectReason.QUEUE_DEPTH);
      }
    });
  });

  describe("metric smoothing", () => {
    it("smooths dependency latency with the configured EWMA alpha", () => {
      const shedder = new AdaptiveLoadShedder({ ewmaAlpha: 0.2 });
      shedder.observeDependencyLatency(100);
      shedder.observeDependencyLatency(200);
      const { latencyMs } = shedder.getMetrics();
      // 0.2*200 + 0.8*100 = 120
      expect(latencyMs).toBeCloseTo(120, 5);
    });
  });
});
