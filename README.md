# Chen Pilot — Autonomous AI Agent for Multi-Chain DeFi
//WIP
Chen Pilot is a sophisticated AI-driven gateway that enables seamless interaction with blockchain networks and DeFi protocols through natural language. It provides a unified, professional interface for managing Bitcoin assets, Stellar operations, cross-chain liquidity swaps, and lending protocols.

---

## Prerequisites

- **Node.js 18+** — required by TypeScript 5.7+, ESLint 9.x
- **PostgreSQL 14+** — required by TypeORM 0.3.x with pg 8.x
- **Redis 6+** — required for the distributed trade-locking system (`src/services/lock`); **not optional**
- **pnpm 8+** — required for monorepo workspace management
- Environment variables configured (see Configuration section)

---

## 🛠️ Installation

### Clone the repository

```bash
git clone <repository-url>
cd chenpilot
```

### Install dependencies

```bash
pnpm install
```

### Set up environment variables

```bash
cp .env.example .env
# Edit .env with your configuration
```

### Set up the database

```bash
npm run migration:run
```

### Start the development server

```bash
npm run dev
```

---

## 📝 Logging System

Chen Pilot includes an automated log rotation system with compression to efficiently manage log files.

### Features

- **Daily Rotation**: Logs automatically rotate at midnight
- **Automatic Compression**: Old logs are compressed to `.gz` format (80-90% size reduction)
- **Size-Based Rotation**: Logs also rotate when reaching 20MB
- **Automatic Cleanup**: Old logs are deleted based on retention policies
- **Sensitive Data Redaction**: Passwords, tokens, and private keys are automatically redacted

### Log Files

- `logs/application-YYYY-MM-DD.log` - All application logs (14 days retention)
- `logs/error-YYYY-MM-DD.log` - Error logs only (30 days retention)
- `logs/exceptions-YYYY-MM-DD.log` - Uncaught exceptions (30 days retention)
- `logs/rejections-YYYY-MM-DD.log` - Unhandled promise rejections (30 days retention)

### Configuration

Set the log level in your `.env` file:

```bash
LOG_LEVEL=info  # Options: debug, info, warn, error
```

### Usage

```typescript
import { logInfo, logError, logWarn, logDebug } from "./config/logger";

logInfo("User logged in", { userId: "123" });
logError("Database error", error, { context: "user-service" });
```

For more details, see [src/config/LOGGING.md](src/config/LOGGING.md)

---

## 🧹 Code Quality & Git Hooks

This project uses **Husky**, **lint-staged**, and **commitlint** to enforce code quality and commit message standards.

### What Runs Automatically

- **pre-commit**
  - Runs ESLint with auto-fix
  - Runs Prettier formatting
  - Blocks commits if checks fail

- **commit-msg**
  - Validates commit messages using Commitlint
  - Enforces Conventional Commits format
  - Blocks commits with invalid commit messages

This setup helps maintain consistent code style and a clean, readable git history.

---

### Local Setup

Git hooks are enabled automatically after installing dependencies:

```bash
pnpm install
```

If hooks do not run for any reason, re-enable Husky manually:

```bash
pnpm exec husky install
```

---

### Configuration Overview

#### pre-commit hook

```sh
pnpm exec lint-staged
```

#### commit-msg hook

```sh
pnpm exec commitlint --edit $1
```

#### lint-staged configuration

```json
{
  "**/*.{ts,tsx}": ["eslint --fix", "prettier --write"]
}
```

#### commitlint configuration

Commit messages must follow the **Conventional Commits** specification.

Examples of valid commit messages:

```text
feat: add cross-chain swap workflow
fix: handle null wallet address
chore: update dependencies
docs: update README
```

---

### Skipping Hooks (Not Recommended)

Hooks can be skipped in exceptional cases:

```bash
git commit --no-verify
```

> ⚠️ Use sparingly. CI checks will still run and invalid commits may be rejected.

---

## Workflow System

The agent uses an intelligent workflow system that:

- **Intent Parsing:** Leverages LLMs to interpret complex financial requests.
- **Strategic Planning:** Generates comprehensive, multi-step `ExecutionPlans` via the **AgentPlanner**.
- **Tool Execution:** Orchestrates specialized tools via the **PlanExecutor**.
- **State Integrity:** Maintains strict tracking of transaction lifecycles and results.
- **User Feedback:** Delivers structured, actionable responses with full transparency.

For visual diagrams of how the **agents**, **tool registry**, and **external services** interact, see `ARCHITECTURE_DIAGRAMS.md` (Mermaid).

---

## SDK Idempotency for BTC → Stellar Swaps

Use the SDK idempotency helpers to guarantee retries do not create duplicate intents.

```ts
import {
  AgentClient,
  createBtcToStellarSwapIdempotencyKey,
  ChainId,
} from "@chen-pilot/sdk-core";

const client = new AgentClient({ baseUrl: "https://your-chenpilot-api" });

const swapRequest = {
  fromChain: ChainId.BITCOIN,
  toChain: ChainId.STELLAR,
  fromToken: "BTC",
  toToken: "XLM",
  amount: "0.01",
  destinationAddress: "G...",
};

// Generate once, persist in your app state, and reuse on retries
const idempotencyKey = createBtcToStellarSwapIdempotencyKey(
  swapRequest,
  "wallet-action-123"
);

await client.executeBtcToStellarSwap(swapRequest, {
  userId: "user-uuid",
  idempotencyKey,
  maxRetries: 3,
});
```

If a timeout/network failure happens, call `executeBtcToStellarSwap` again with the same `idempotencyKey`.

---

## SDK API Docs (TypeDoc)

The SDK package includes TypeDoc configuration for automated HTML API reference generation.

### Generate docs locally

```bash
pnpm install
pnpm --dir packages/sdk run docs
```

Generated output is written to:

```bash
packages/sdk/docs
```

### Automated publishing on push to `main`

GitHub Actions workflow: `.github/workflows/sdk-docs.yml`

On each push to `main`, the workflow:

1. Installs SDK dependencies
2. Runs `npm run docs` in `packages/sdk`
3. Deploys the generated HTML docs to GitHub Pages

---

## Distributed Locking for Trade Execution

The system uses Redis-based distributed locks to prevent race conditions when executing multiple trades for the same user concurrently.

### How it works

- **Lock Key Pattern**: `trade:{userId}` - Ensures one trade per user at a time
- **Lock TTL**: 60 seconds with configurable retry strategy
- **Atomic Operations**: Uses Redis SET with NX/EX and Lua scripts for safe lock/release
- **Automatic Cleanup**: Locks auto-expire to prevent deadlocks

### Lock Service Features

- **Acquire Lock**: `acquireLock(resourceKey, identifier, options)`
- **Release Lock**: `releaseLock(resourceKey, identifier)` - Only releases if owned by identifier
- **Extend Lock**: `extendLock(resourceKey, identifier, ttl)` - For long-running operations
- **Status Check**: `isLocked(resourceKey)` and `getLockInfo(resourceKey)`

### Integration in Swap Tool

The `SwapTool` automatically acquires a user-specific lock before executing trades:

```typescript
const lockKey = `trade:${userId}`;
const lockResult = await lockService.acquireLock(lockKey, userId, {
  ttl: 60000,
  retryDelay: 200,
  maxRetries: 15,
});

if (!lockResult.acquired) {
  return this.createErrorResult(
    "swap",
    "An active trade execution is already in progress for this account. Chen Pilot enforces a single-session lock to ensure transaction integrity and security. Please wait for the current operation to finalize."
  );
}

// Execute trade with lock held...
// Lock is automatically released in finally block
```

### Configuration

Redis connection is configured via environment variables:

```bash
REDIS_HOST=localhost
REDIS_PORT=6379
REDIS_PASSWORD=optional
REDIS_DB=0
```

### Error Handling

- **Lock Acquisition Failure**: Returns user-friendly error message
- **Redis Connection Issues**: Graceful fallback with error logging
- **Lock Extension**: Supports long-running operations beyond initial TTL
- **Automatic Release**: Locks auto-expire to prevent permanent locks

---

## Health Endpoints

Chen Pilot exposes two operations-facing probes with different purposes:

- `GET /health` is a liveness probe. It returns HTTP `200` with a lightweight
  `{ status, timestamp }` payload while the process is up.
- `GET /ready` is a readiness probe. It returns a full dependency report and
  uses HTTP `503` when a critical dependency is down.

The current readiness report includes:

- `database`
- `redis`
- `horizon`
- `sorobanRpc`
- `email`
- `llm`

Each dependency includes `status`, `latencyMs`, and optional `error` or
`detail` fields. `database` and `redis` are treated as critical. Redis and
Soroban RPC connectivity are therefore visible directly in the readiness
payload, rather than being collapsed into a single boolean.

For contributor-facing operational detail, see
`docs/SYSTEMS_HANDBOOK.md#health-and-readiness-endpoints`.

## Rate Limiting In Multi-Instance Deployments

Gateway rate limiting is configured in
`src/Gateway/middleware/rateLimiter.service.ts`.

- The primary `express-rate-limit` store is Redis-backed via `rate-limit-redis`
  and `getRedisClient()`.
- This allows request counters to be shared across multiple API instances when
  they point at the same Redis deployment.
- If Redis-backed limiter creation fails, the service currently falls back to
  the default in-memory store. That keeps one node serving traffic, but it no
  longer enforces a shared limit across a horizontally scaled fleet.

Operators should treat the in-memory fallback as a degraded mode and restore
Redis connectivity before relying on cluster-wide request ceilings.

---

## Contributing

- Fork the repository
- Create a feature branch
- Make your changes
- Ensure pre-commit and commit message checks pass
- Add tests if applicable
- Submit a pull request

---

## License

This project is licensed under the ISC License.

---

## Support

For technical support and community inquiries:

- Create an issue in the repository
- Monitor `/health` for liveness and `/ready` for dependency-level readiness
- Review the logs for error details

---

Chen Pilot — Your intelligent gateway to cross-chain DeFi operations

```

```
