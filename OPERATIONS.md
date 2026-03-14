# Operations Manual — nonthok-cashflow Options Wheel

## 1. System Overview

This system is a **Rust-based trading bot** that implements the **Options Wheel strategy** on a single underlying equity (default: BAC — Bank of America). It runs on a scheduled basis (e.g., via cron), advances a persistent state machine one step per invocation, and generates cash flow through repeated option premium collection.

**Core strategy:**
- Sell a cash-secured put (CSP) → collect premium
- If assigned: sell a covered call (CC) → collect more premium
- If called away: restart the cycle

The system uses Alpaca Markets' paper trading API by default. All trades are placed as limit orders with automatic market-order fallback.

---

## 2. Daily Operations Schedule

The bot is **scheduler-driven** — it does not run continuously. Each invocation performs exactly one state machine step.

### Recommended Cron Schedule

```
# Run daily at 10:00 AM ET (during market hours) Monday–Friday
0 10 * * 1-5 cd /path/to/nonthok-cashflow && ./target/release/options-wheel >> /var/log/wheel.log 2>&1
```

### What Happens Each Run

| Time | Action |
|------|--------|
| Run triggered | Load config from environment variables |
| | Validate account (options approval level, buying power) |
| | Fetch underlying (BAC) bid/ask quote |
| | Load state from `~/.wheel_state.json` |
| | Advance state machine by one step |
| | Save new state to `~/.wheel_state.json` |
| | Exit |

**Market hours:** The Alpaca API operates during normal US market hours (9:30 AM – 4:00 PM ET). Running during market hours ensures options chains and quotes are current. Running outside market hours may return stale data or fail.

**No intraday monitoring:** The bot does not run continuously. It checks positions and advances the state once per scheduled run. Between runs, positions are unmonitored.

---

## 3. Wheel State Machine

The bot persists its current state in `~/.wheel_state.json`. Each run loads, advances, and saves this state.

```
    ┌─────────────────────────────────────────────────────────┐
    │                                                         │
    ▼                                                         │
  Idle ──────────────────────────────────► WatchingCSP        │
  (sell CSP)                              (monitor short put) │
                                                 │            │
                               ┌─────────────────┤            │
                               │                 │            │
                          Expired/50%        Assigned         │
                           (keep premium)    (got shares)     │
                               │                 │            │
                               └──► Idle ◄──     ▼            │
                                            AssignedLong       │
                                            (sell CC)         │
                                                 │            │
                                                 ▼            │
                                           WatchingCC         │
                                           (monitor CC)       │
                                                 │            │
                               ┌─────────────────┤            │
                               │                 │            │
                          Expired/50%       Called Away       │
                          (keep shares,  (shares sold)        │
                           sell more CC)      │               │
                               │              ▼               │
                               └──► AssignedLong    Called ───┘
                                                    (restart)
```

### States

| State | Description | Entry Condition | Exit Conditions |
|-------|-------------|-----------------|-----------------|
| **Idle** | No open position. Ready to start a cycle. | Initial, or after cycle completes | Always transitions to WatchingCSP |
| **WatchingCSP** | Monitoring a short put. | CSP order placed and filled | Expired → Idle; Assigned → AssignedLong; 50% profit → Idle |
| **AssignedLong** | Holding 100+ shares of BAC from put assignment. | Put assignment detected | Always transitions to WatchingCC (after CC is sold) |
| **WatchingCC** | Monitoring a short call. | CC order placed and filled | Expired → AssignedLong (sell another CC); Called → Called; 50% profit → AssignedLong |
| **Called** | Shares called away by CC assignment. | CC assignment detected | Immediately transitions to Idle |

### State File

```json
// ~/.wheel_state.json examples:
{ "state": "Idle" }
{ "state": "WatchingCSP", "order_id": "abc123", "symbol": "BAC260404P00038000" }
{ "state": "AssignedLong", "entry_cost": 3812.00 }
{ "state": "WatchingCC", "order_id": "xyz789", "symbol": "BAC260515C00042000" }
{ "state": "Called", "realized_pnl": 0.0 }
```

To reset the state machine: `rm ~/.wheel_state.json`

---

## 4. Strike Selection Logic

Applies to both CSP and CC selection. The same filter pipeline is used for both puts and calls.

### Filter Pipeline

1. **Delta range:** `0.20 ≤ |delta| ≤ 0.25`
   - Targets the 20–25 delta range for a ~75–80% probability of expiring worthless
   - For puts: delta is negative (e.g., −0.22); for calls: positive (e.g., +0.22)

2. **Open interest:** `OI ≥ 200 contracts`
   - Filters illiquid strikes with unreliable bid/ask pricing

3. **Bid-ask spread:** `ask − bid ≤ $0.10`
   - Ensures tight, fillable markets

4. **Earnings avoidance:** Skips contracts expiring within **−2 to +7 days** of a known BAC earnings date
   - Avoids IV crush and gap risk around earnings announcements
   - 2026 earnings dates hardcoded: Jan 14, Apr 15, Jul 15, Oct 14

5. **Selection:** Among all passing contracts, picks the one with the **highest mid-price** `(bid + ask) / 2`
   - Maximizes premium collected

### DTE Window

- Default: **21–30 days to expiration** (configurable via `TARGET_DTE_MIN` / `TARGET_DTE_MAX`)
- Targets the "sweet spot" for theta decay while maintaining meaningful premium

### OCC Symbol Format

Options symbols follow the OCC standard:

```
BAC 260404 P 00038000
 ↑    ↑    ↑    ↑
 │    │    │    └── Strike × 1000, zero-padded to 8 digits ($38.00)
 │    │    └─────── Contract type: P=put, C=call
 │    └──────────── Expiration: YYMMDD (2026-04-04)
 └───────────────── Underlying symbol
```

---

## 5. Order Execution

### Placement Strategy

All option orders use a **limit-first, market-fallback** approach:

1. Place a **limit order** at the current mid-price
2. Poll every **10 seconds** for up to **60 seconds**
3. If not filled after 60 seconds: cancel the limit order and place a **market order**
4. Poll the market order every 10 seconds for up to 60 seconds
5. If the market order also fails: the run exits with an error (state is not advanced)

### Pre-flight Checks

**Before placing a CSP:**
- Verifies `options_buying_power ≥ strike_price × 100`
- Prevents over-leveraging

**Before placing a CC:**
- Verifies equity position exists with `qty ≥ 100` shares
- Ensures shares are actually held before selling a covered call

### Order Types Used

| Order | Side | Type | TIF |
|-------|------|------|-----|
| Sell CSP | Sell | Limit → Market | Day |
| Sell CC | Sell | Limit → Market | Day |
| Buy-to-close | Buy | Limit → Market | Day |

---

## 6. Assignment Detection

### Paper Trading Limitation

Alpaca paper trading accounts do **not** deliver real-time assignment notifications (NTA events). These are typically delayed until the following business day. The bot works around this by:

**Primary method:** Check directly for an equity position in BAC with `qty ≥ 100`. If shares exist after a put expiration, the put was assigned.

**Fallback method:** Monitor account activity events for:
- `OPASN` — Option assignment
- `OPEXP` — Option expiration (worthless)
- `OPEXC` — Option exercise

### Assignment Flow

```
WatchingCSP run:
  1. Call detect_wheel_event(client, "BAC")
  2. If BAC equity position qty >= 100 → WheelEvent::Assigned → go to AssignedLong
  3. If no option position and no equity → WheelEvent::Expired → go to Idle
  4. If option position still open → WheelEvent::Active → stay in WatchingCSP
```

---

## 7. Profit-Taking (50% Early Close)

When a position's unrealized P&L reaches **≥ 50%** of initial premium collected, the bot automatically buys to close:

- **CSP (WatchingCSP):** Buy-to-close the short put → go to Idle
- **CC (WatchingCC):** Buy-to-close the short call → go back to AssignedLong (shares still held)

This is checked on every run while in WatchingCSP or WatchingCC state.

```
If position.unrealized_plpc >= 0.50:
    place_buy_to_close(symbol, current_price)
    → state machine advances
```

The 50% rule frees capital faster and reduces time-in-trade, allowing more cycles per month.

---

## 8. Risk Controls

| Control | Value | Description |
|---------|-------|-------------|
| Buying power cap | `MAX_BUYING_POWER` (default $5,000) | Upper limit on options buying power deployed |
| Pre-flight BP check | `strike × 100` | Must have enough BP before placing CSP |
| Pre-flight equity check | `qty ≥ 100` | Must hold shares before placing CC |
| Options approval | Level ≥ 1 | Account must have options trading enabled |
| Earnings avoidance | ±7 days | Skip contracts near BAC earnings dates |
| Paper mode default | `APCA_API_BASE_URL` = paper API | No real money at risk |

---

## 9. Configuration

All configuration is loaded from environment variables. A `.env` file in the working directory is loaded automatically if present.

### Required

| Variable | Description |
|----------|-------------|
| `APCA_API_KEY_ID` | Alpaca API key (paper or live) |
| `APCA_API_SECRET_KEY` | Alpaca API secret |

### Optional (with defaults)

| Variable | Default | Description |
|----------|---------|-------------|
| `APCA_API_BASE_URL` | `https://paper-api.alpaca.markets` | API base URL. Use `https://api.alpaca.markets` for live trading |
| `APCA_DATA_URL` | `https://data.alpaca.markets` | Market data URL |
| `MAX_BUYING_POWER` | `5000.0` | Maximum options buying power to deploy (USD) |
| `UNDERLYING` | `BAC` | Underlying symbol to trade |
| `TARGET_DTE_MIN` | `21` | Minimum days to expiration for option selection |
| `TARGET_DTE_MAX` | `30` | Maximum days to expiration for option selection |
| `RUST_LOG` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |

### Example `.env` file

```env
APCA_API_KEY_ID=your_paper_key_here
APCA_API_SECRET_KEY=your_paper_secret_here
APCA_API_BASE_URL=https://paper-api.alpaca.markets
MAX_BUYING_POWER=10000.0
UNDERLYING=BAC
TARGET_DTE_MIN=21
TARGET_DTE_MAX=30
RUST_LOG=info
```

---

## 10. Paper Trading Limitations

| Limitation | Impact | Workaround |
|------------|--------|------------|
| Assignment events delayed until next business day | Bot won't detect assignment on expiration day | Check equity position directly (already implemented) |
| Indicative options feed (not real-time) | Prices may lag live market | Use `feed=indicative` in chain requests |
| Paper fills may differ from live | Market orders may fill at unrealistic prices | Use limit orders first (already the default) |
| Options chain availability | May be empty on weekends/holidays | Run only on market days |

---

## 11. Monitoring & Logging

### Log Format

The bot uses structured logging via the `tracing` crate. Each line includes timestamp, level, and context fields.

### Key Log Lines to Watch

```
# Startup
Options Wheel starting (paper mode)  underlying=BAC  max_buying_power=10000  dte_window=21-30

# Account health
Account verified  options_buying_power=48231.44  options_approved_level=2
Effective buying power  effective_buying_power=10000.0

# State machine
[WHEEL] Loaded state: Idle
[WHEEL] State: Idle — fetching options chain
[WHEEL] Idle -> WatchingCSP: BAC260404P00038000, delta=-0.22, premium=$0.74
[ORDER] SELL PUT BAC260404P00038000 x1 @ $0.74 LIMIT -> filled @ $0.74

# Position monitoring
[PNL] Position P&L: +37 (50% of initial premium), DTE=14
[WHEEL] WatchingCSP: 50%+ profit captured (51%), triggering buy-to-close
[ORDER] BUY BAC260404P00038000 x1 @ $0.37 LIMIT -> filled @ $0.37 (50% profit close)

# Assignment
[WHEEL] WatchingCSP -> AssignedLong: 100 shares @ $38.12/share, total cost $3812.00

# Errors
Config error: APCA_API_KEY_ID not set
Options trading not approved on this account (level=0). Enable options in Alpaca account settings.
Insufficient options buying power: $2000.00 available, $3800.00 required for BAC260404P00038000 CSP
No CSP candidates pass all filters (delta 0.20-0.25, OI>=200, spread<=0.10, no earnings).
```

### Log Levels

| Level | Use |
|-------|-----|
| `error` | Fatal conditions, config failures |
| `warn` | Non-fatal issues (e.g., limit order timeout, chain fetch failure) |
| `info` | Normal operation: state transitions, order placements, fills |
| `debug` | Detailed API responses |
| `trace` | Full request/response bodies |

Set `RUST_LOG=debug` to diagnose order or chain issues.

---

## 12. Building and Running

### Prerequisites

- Rust toolchain (stable, 1.75+): https://rustup.rs
- Alpaca paper trading account with options level ≥ 1 enabled

### Build

```bash
git clone https://github.com/nonthok-cashflow/nonthok-cashflow
cd nonthok-cashflow
cargo build --release -p options-wheel
```

### Run

```bash
# Set environment variables
export APCA_API_KEY_ID=your_key
export APCA_API_SECRET_KEY=your_secret
export MAX_BUYING_POWER=10000.0

# Run one wheel step
./target/release/options-wheel

# Or with a .env file:
cp .env.example .env   # edit with your credentials
./target/release/options-wheel
```

### Integration Tests

```bash
# Requires real paper trading credentials in environment
cargo test -p options-wheel --test integration -- --nocapture
```

Tests included:
- `test_full_csp_cycle` — places and cancels a CSP order end-to-end
- `test_account_health` — validates account approval level and buying power
- `test_fetch_options_chain` — fetches and filters the BAC options chain
