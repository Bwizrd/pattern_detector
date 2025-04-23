#!/bin/bash

# Script to compare Generator vs API active zone lists from logs

OUTPUT_FILE="comparison_output.txt"
LOG_FILE="backend.log" # Make sure this points to your actual log file

# Check if log file exists
if [ ! -f "$LOG_FILE" ]; then
    echo "Error: Log file '$LOG_FILE' not found!"
    exit 1
fi

echo "Starting comparison..."
echo "Comparing Generator vs API Active Lists from $LOG_FILE" > "$OUTPUT_FILE"
echo "======================================================" >> "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"

# Use a 'here document' (<<EOF) to define the list of zones to check
# Format: Symbol Timeframe StartTime (space-separated)
while read -r symbol timeframe start_time; do
    # Skip empty lines or lines starting with #
    [[ -z "$symbol" || "$symbol" == \#* ]] && continue

    # Construct the symbol variations needed for grep
    # Assuming Generator logs use _SB suffix and API logs do not
    symbol_sb="${symbol}_SB"

    echo "--- Comparison for: ${symbol}/${timeframe} (StartTime: ${start_time}) ---" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE" # Add blank line

    # Grep for Generator log entry
    echo "[GENERATOR]" >> "$OUTPUT_FILE"
    # Ensure grep doesn't output error messages if nothing is found, just empty output
    grep "\[GENERATOR_ACTIVE_LIST\] ${symbol_sb}/${timeframe}:" "$LOG_FILE" >> "$OUTPUT_FILE" || echo "  (No matching Generator log line found)" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE"

    # Grep for API log entry
    echo "[API PRE-FILTER]" >> "$OUTPUT_FILE"
    grep "\[API_ACTIVE_LIST\] \[Task ${symbol}/${timeframe}\]" "$LOG_FILE" >> "$OUTPUT_FILE" || echo "  (No matching API log line found)" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE"

    echo "-------------------------------------------------------------" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE"

done <<EOF
# List of the 18 potentially missing zones (Symbol Timeframe StartTime)
AUDCAD 30m 2025-04-03T14:30:00Z
AUDJPY 1h 2025-04-04T01:00:00Z
AUDJPY 5m 2025-04-16T17:25:00Z
AUDNZD 15m 2025-04-10T00:00:00Z
AUDNZD 30m 2025-04-09T23:30:00Z
AUDUSD 5m 2025-04-11T08:40:00Z
CADJPY 15m 2025-04-10T07:15:00Z
CHFJPY 15m 2025-04-15T12:00:00Z
CHFJPY 5m 2025-04-11T17:40:00Z
EURAUD 1h 2025-04-09T00:00:00Z
EURCAD 15m 2025-03-05T07:15:00Z
EURNZD 15m 2025-04-14T12:00:00Z
GBPCAD 15m 2025-04-11T00:15:00Z
GBPJPY 15m 2025-04-04T07:15:00Z
GBPNZD 5m 2025-04-09T12:05:00Z
GBPUSD 15m 2025-03-04T08:00:00Z
USDCHF 15m 2025-04-10T14:30:00Z
USDJPY 15m 2025-04-16T17:15:00Z
EOF

echo "Comparison complete. Results are in ${OUTPUT_FILE}"