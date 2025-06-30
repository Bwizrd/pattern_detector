# Combined Backtest Endpoint

## Overview

The `/combined-backtest` endpoint provides a reliable way to backtest multiple currency pairs using the same proven logic as the single `/backtest` endpoint. This endpoint was created to replace the problematic `/multi-symbol-backtest` endpoint which had inconsistent results due to mixing old and new zone detection methods.

## Key Features

- **Consistent Results**: Uses the same `use_old_method=true` logic as the single backtest endpoint
- **Multiple Currency Pairs**: Test multiple symbols in one request
- **Default Symbol List**: If no symbols provided, tests 16 major currency pairs
- **Comprehensive Analysis**: Returns detailed statistics per symbol and overall summary
- **Professional Reporting**: Includes trade distribution, win rates, and profit factors
- **Environment Variable Support**: Falls back to environment variables for trading parameters

## Endpoint Details

**URL**: `POST /combined-backtest`

**Content-Type**: `application/json`

## Request Format

```json
{
  "symbols": ["EURUSD", "GBPUSD", "USDJPY"],          // Optional - defaults to 16 major pairs
  "patternTimeframes": ["1h", "4h", "1d"],            // Required - multiple timeframes to test
  "startTime": "2024-01-01T00:00:00Z",               // Required - RFC3339 format
  "endTime": "2024-12-31T23:59:59Z",                 // Required - RFC3339 format
  "pattern": "fifty_percent_before_big_bar",          // Required
  
  // Trading Parameters (optional - fall back to env vars)
  "lotSize": 0.01,
  "stopLossPips": 20.0,
  "takeProfitPips": 10.0,
  
  // Trading Rules (optional - fall back to env vars)
  "maxTradesPerZone": 3,
  "showRejectedTrades": false
}
```

### Default Currency Pairs

If `symbols` array is empty, the endpoint tests these 16 major pairs:
- EURUSD, GBPUSD, USDJPY, USDCHF
- AUDUSD, USDCAD, NZDUSD, EURGBP
- EURJPY, GBPJPY, AUDJPY, CHFJPY
- EURCHF, EURAUD, GBPCHF, GBPAUD

## Response Format

```json
{
  "overall_summary": {
    "total_trades": 150,
    "total_pnl_pips": 245.8,
    "overall_win_rate_percent": 68.5,
    "overall_profit_factor": 2.35,
    "average_trade_duration_str": "4.2 hours",
    "overall_winning_trades": 102,
    "overall_losing_trades": 48,
    "trades_by_symbol_percent": {
      "EURUSD": 18.7,
      "GBPUSD": 15.3,
      "USDJPY": 12.0
    },
    "trades_by_timeframe_percent": {
      "1h": 100.0
    },
    "trades_by_day_of_week_percent": {
      "Monday": 22.0,
      "Tuesday": 18.5
    },
    "pnl_by_day_of_week_pips": {
      "Monday": 54.2,
      "Tuesday": 31.8
    },
    "trades_by_hour_of_day_percent": {
      "9": 15.2,
      "14": 12.8
    },
    "pnl_by_hour_of_day_pips": {
      "9": 28.5,
      "14": 22.1
    }
  },
  "detailed_summaries": [
    {
      "symbol": "EURUSD",
      "timeframe": "1h",
      "total_trades": 28,
      "winning_trades": 19,
      "losing_trades": 9,
      "win_rate_percent": 67.9,
      "total_pnl_pips": 42.5,
      "profit_factor": 2.1
    }
  ],
  "all_trades": [
    {
      "symbol": "EURUSD",
      "timeframe": "1h",
      "zone_id": "EURUSD_1h_supply_1704067200",
      "direction": "Short",
      "entry_time": "2024-01-01T08:00:00Z",
      "entry_price": 1.1050,
      "exit_time": "2024-01-01T12:00:00Z",
      "exit_price": 1.1030,
      "pnl_pips": 20.0,
      "exit_reason": "Take Profit",
      "zone_strength": 145.2,
      "touch_count": 2,
      "entry_day_of_week": "Monday",
      "entry_hour_of_day": 8
    }
  ],
  "trading_rules_applied": {
    "trading_enabled": true,
    "lot_size": 0.01,
    "default_stop_loss_pips": 20.0,
    "default_take_profit_pips": 10.0,
    "max_trades_per_zone": 3
  },
  "environment_variables": {
    "trading_stop_loss_pips": "20",
    "trading_take_profit_pips": "10"
  },
  "request_parameters": {
    "symbols": ["EURUSD", "GBPUSD"],
    "pattern_timeframes": ["1h"],
    "start_time": "2024-01-01T00:00:00Z",
    "end_time": "2024-12-31T23:59:59Z"
  }
}
```

## Advantages Over Multi-Symbol Backtest

1. **Consistent Logic**: Uses `use_old_method=true` for all symbols, ensuring consistency with single backtest
2. **No Zone Detection Conflicts**: Avoids mixing ZoneDetectionEngine with old pattern recognizers
3. **Reliable Results**: Each symbol processed using identical logic to single backtest
4. **Simplified Implementation**: Cleaner code without redundant zone detection steps
5. **Better Error Handling**: Individual symbol failures don't affect other symbols

## Example Usage

### Test Default Major Pairs on Single Timeframe
```bash
curl -X POST http://localhost:8080/combined-backtest \
  -H "Content-Type: application/json" \
  -d '{
    "symbols": [],
    "patternTimeframes": ["1h"],
    "startTime": "2024-01-01T00:00:00Z",
    "endTime": "2024-03-31T23:59:59Z",
    "pattern": "fifty_percent_before_big_bar"
  }'
```

### Test Multiple Timeframes with Custom Parameters
```bash
curl -X POST http://localhost:8080/combined-backtest \
  -H "Content-Type: application/json" \
  -d '{
    "symbols": ["EURUSD", "GBPUSD", "USDJPY"],
    "patternTimeframes": ["1h", "4h", "1d"],
    "startTime": "2024-01-01T00:00:00Z",
    "endTime": "2024-12-31T23:59:59Z",
    "pattern": "fifty_percent_before_big_bar",
    "lotSize": 0.02,
    "stopLossPips": 30.0,
    "takeProfitPips": 15.0,
    "maxTradesPerZone": 2,
    "showRejectedTrades": true
  }'
```

### Test All Major Pairs Across Multiple Timeframes
```bash
curl -X POST http://localhost:8080/combined-backtest \
  -H "Content-Type: application/json" \
  -d '{
    "symbols": [],
    "patternTimeframes": ["30m", "1h", "4h"],
    "startTime": "2024-01-01T00:00:00Z",
    "endTime": "2024-06-30T23:59:59Z",
    "pattern": "fifty_percent_before_big_bar",
    "stopLossPips": 25.0,
    "takeProfitPips": 12.5
  }'
```

## Supported Patterns

- `fifty_percent_before_big_bar` (Primary)
- `price_sma_cross`
- `specific_time_entry`

## Environment Variables

The endpoint falls back to these environment variables if parameters aren't provided:

- `TRADING_STOP_LOSS_PIPS` (default: 20.0)
- `TRADING_TAKE_PROFIT_PIPS` (default: 10.0)
- `TRADING_LOT_SIZE` (default: 10, converted to 0.01)
- `TRADING_MAX_TRADES_PER_ZONE` (default: 3)

## Performance

- Processes symbols sequentially for reliability
- Uses same optimized data loading as single backtest
- Includes 1-minute data for precise execution when available
- Comprehensive error handling and logging

## Migration from Multi-Symbol Backtest

If you're currently using `/multi-symbol-backtest`, you can migrate to `/combined-backtest` by:

1. Changing the endpoint URL
2. Updating request structure (minimal changes needed)
3. Expecting the same response format but with reliable results
4. Planning to remove the old `/multi-symbol-backtest` endpoint in future versions

This endpoint provides the foundation for eventually deprecating the problematic multi-symbol implementation.
