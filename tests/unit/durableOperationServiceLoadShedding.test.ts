import { describe, it, expect, jest, beforeEach } from "@jest/globals";

const repoMock = {
  findOne: jest.fn<any>(),
  find: jest.fn<any>(),
  save: jest.fn<any>(),
  createQueryBuilder: jest.fn<any>(),
};

jest.mock("../../src/config/Datasource", () => ({
  __esModule: true,
  default: { getRepository: jest.fn(() => repoMock) },
  AppDataSource: { getRepository: jest.fn(() => repoMock) },
}));

jest.mock("../../src/config/logger", () => ({
  __esModule: true,
  default: {
    info: jest.fn(),
    warn: jest.fn(),
    error: jest.fn(),
    debug: jest.fn(),
  },
}));

jest.mock("typeorm", () => {
  const actual = jest.requireActual("typeorm");
  return {
    ...actual,
    LessThanOrEqual: (v: unknown) => v,
    IsNull: () => null,
    Or: (...args: unknown[]) => args,
  };
});

import { DurableOperationService } from "../../src/Reliability/DurableOperationService";
import { loadShedder, TrafficClass } from "../../src/Reliability/AdaptiveLoadShedder";

/** Statuses observed on each persisted save, captured at call time. */
let savedStatuses: string[] = [];

function makeOp(overrides: Partial<any> = {}) {
  return {
    id: "op-1",
    category: "test",
    payload: {},
    status: "pending",
    retries: 0,
    maxRetries: 3,
    ...overrides,
  };
}

/** Configure the shared repo mock so `save` snapshots each status at call time. */
function setupRepo(ops: any[]) {
  savedStatuses = [];
  repoMock.find.mockResolvedValue(ops);
  repoMock.findOne.mockResolvedValue(ops[0]);
  // Capture the status when save() is invoked. We must snapshot the value here
  // rather than reading repoMock.save.mock.calls afterwards, because the
  // service reuses the same operation object across its RUNNING -> COMPLETED
  // transitions and mutates it in place.
  repoMock.save.mockImplementation((o: any) => {
    if (o && o.status) savedStatuses.push(o.status);
    return Promise.resolve(o);
  });
  return new DurableOperationService();
}

beforeEach(() => {
  loadShedder.reset();
  jest.clearAllMocks();
});

function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 20));
}

describe("DurableOperationService adaptive load shedding integration", () => {
  it("sheds (defers) new execution work when the system is overloaded", async () => {
    // Force the shedder into shedding mode via high dependency latency.
    loadShedder.observeDependencyLatency(1000);

    const service = setupRepo([makeOp({ id: "exec-1", nextRetryAt: undefined })]);
    service.registerHandler("test", async () => ({}));
    await service.processPendingOperations();
    await flush();

    // Shedding must be active...
    expect(loadShedder.getMode()).toBe("shedding");
    // ...and the new-execution op must be deferred: it never transitions to
    // RUNNING, so runOperation was never triggered on it.
    expect(savedStatuses.includes("running")).toBe(false);
  });

  it("admits recovery work even while shedding (reserved capacity)", async () => {
    loadShedder.observeDependencyLatency(1000);

    const service = setupRepo([makeOp({ id: "rec-1", nextRetryAt: new Date(Date.now() - 1000) })]);
    service.registerHandler("test", async () => ({}));
    await service.processPendingOperations();
    await flush();

    // Recovery class retains its reserved slots during shedding, so the op is
    // transitioned to RUNNING and executed.
    expect(savedStatuses.includes("running")).toBe(true);
  });

  it("returns zero in-flight after each processed batch (no slot leak)", async () => {
    loadShedder.observeDependencyLatency(1000);

    const service = setupRepo([makeOp({ id: "rec-2", nextRetryAt: new Date(Date.now() - 1000) })]);
    service.registerHandler("test", async () => ({}));
    await service.processPendingOperations();
    await flush();

    const admitted = loadShedder.admit(TrafficClass.RECOVERY);
    expect(admitted.allowed).toBe(true);
  });

  it("runs new execution work normally when healthy", async () => {
    const service = setupRepo([makeOp({ id: "exec-2", nextRetryAt: undefined })]);
    service.registerHandler("test", async () => ({}));
    await service.processPendingOperations();
    await flush();

    expect(savedStatuses.includes("running")).toBe(true);
  });
});
