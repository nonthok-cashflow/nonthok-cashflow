# nonthok-cashflow — Options Wheel Bot

A Rust-based trading bot that implements the **Options Wheel strategy** on Alpaca Markets paper and live accounts. Generates cash flow by repeatedly selling cash-secured puts and covered calls on a target underlying (default: BAC — Bank of America).

> **Default mode: paper trading.** No real money is at risk until you explicitly configure live API credentials.

---

## Quick Start

### 1. Prerequisites

- Rust (stable, 1.75+) — install from [rustup.rs](https://rustup.rs)
- An [Alpaca Markets](https://alpaca.markets) paper trading account
- Options trading enabled on your Alpaca account (level 1 or higher)

### 2. Clone & Build

```bash
git clone https://github.com/nonthok-cashflow/nonthok-cashflow
cd nonthok-cashflow
cargo build --release -p options-wheel
```

### 3. Configure

```bash
# Create a .env file with your paper trading credentials
cat > .env << EOF
APCA_API_KEY_ID=your_paper_api_key
APCA_API_SECRET_KEY=your_paper_api_secret
APCA_API_BASE_URL=https://paper-api.alpaca.markets
MAX_BUYING_POWER=10000.0
UNDERLYING=BAC
TARGET_DTE_MIN=21
TARGET_DTE_MAX=30
RUST_LOG=info
EOF
```

### 4. Run (one wheel step)

```bash
./target/release/options-wheel
```

Each invocation advances the state machine by one step: sells a put, monitors a position, closes for profit, etc. Run on a schedule (e.g., daily cron during market hours) to drive the full cycle.

### 5. Schedule (cron example)

```cron
# Run daily at 10:00 AM ET, Monday–Friday
0 10 * * 1-5 cd /path/to/nonthok-cashflow && ./target/release/options-wheel >> /var/log/wheel.log 2>&1
```

---

## How It Works

The bot cycles through four states:

```
Idle → WatchingCSP → AssignedLong → WatchingCC → Idle → ...
```

1. **Idle:** Sells a cash-secured put (20–25 delta, 21–30 DTE) to collect premium
2. **WatchingCSP:** Monitors the short put; buys to close at 50% profit, or transitions on assignment/expiration
3. **AssignedLong:** Acquired 100 shares; immediately sells a covered call
4. **WatchingCC:** Monitors the short call; buys to close at 50% profit, or cycles back on expiration/assignment

State is persisted to `~/.wheel_state.json` between runs.

---

## Environment Variables

| Variable | Default | Required | Description |
|----------|---------|----------|-------------|
| `APCA_API_KEY_ID` | — | ✅ | Alpaca API key |
| `APCA_API_SECRET_KEY` | — | ✅ | Alpaca API secret |
| `APCA_API_BASE_URL` | `https://paper-api.alpaca.markets` | | API URL (change to live when ready) |
| `APCA_DATA_URL` | `https://data.alpaca.markets` | | Market data URL |
| `MAX_BUYING_POWER` | `5000.0` | | Maximum capital to deploy (USD) |
| `UNDERLYING` | `BAC` | | Symbol to trade |
| `TARGET_DTE_MIN` | `21` | | Minimum days to expiration |
| `TARGET_DTE_MAX` | `30` | | Maximum days to expiration |
| `RUST_LOG` | `info` | | Log level (`trace`/`debug`/`info`/`warn`/`error`) |

---

## Testing

```bash
# Unit tests (no credentials needed)
cargo test

# Integration tests (requires paper trading credentials in environment)
cargo test -p options-wheel --test integration -- --nocapture
```

---

## Full Documentation

See [OPERATIONS.md](./OPERATIONS.md) for the complete operations manual:

- Daily schedule and cron setup
- Full state machine reference
- Strike selection logic and filters
- Order execution and fill monitoring
- Assignment detection (including paper trading workarounds)
- Profit-taking rules
- Risk controls
- All configuration options
- Monitoring and log interpretation

---

## Project Structure

```
nonthok-cashflow/
├── crates/
│   ├── alpaca-client/   # Alpaca REST + WebSocket API client
│   ├── config/          # Configuration loading
│   ├── trading/         # Order execution framework
│   └── options-wheel/   # Main strategy (state machine, chain selection, orders)
│       └── src/
│           ├── main.rs      # Entry point
│           ├── wheel.rs     # State machine
│           ├── chain.rs     # Options chain fetcher + strike selector
│           ├── orders.rs    # Order placement + fill monitoring
│           ├── positions.rs # Position tracking + assignment detection
│           ├── account.rs   # Account validation
│           └── config.rs    # Strategy configuration
├── OPERATIONS.md        # Full operations manual
└── README.md            # This file
```

---

## Safety

- Always start with paper trading (`APCA_API_BASE_URL=https://paper-api.alpaca.markets`)
- Review all open positions before switching to live credentials
- The bot places real orders — ensure you understand the strategy before going live
- State is stored locally in `~/.wheel_state.json`; back it up or delete it to reset
