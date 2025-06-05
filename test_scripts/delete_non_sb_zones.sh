#!/bin/bash

# --- Configuration ---
ORG="PansHouse"
BUCKET="market_data"
TOKEN="VNC3xnPXodbpC3yJ_riWrBpN0lCA0k-mPiFsocR-Wu9K8kFHQ3JUp32bOCQaNOdjVI6zfGuxoZpgGZl-ZiXP-Q==" # Your InfluxDB Token
START_TIME="1970-01-01T00:00:00Z"
# Get the current UTC time once for consistency across all delete operations
# Ensure 'date' command is available and works as expected in your environment
# For macOS/Linux:
STOP_TIME=$(date -u +'%Y-%m-%dT%H:%M:%SZ')
# If using Git Bash on Windows or another environment where the above date command might differ,
# you might need to adjust or use a fixed recent future timestamp.

# Array of non-SB symbols to delete
# This list comes from the distinct symbols you identified earlier
SYMBOLS_TO_DELETE=(
    "AUDCAD" "AUDNZD" "AUDUSD" "CADJPY" "CHFJPY" "EURGBP" "EURJPY" "EURUSD"
    "GBPAUD" "GBPCAD" "GBPCHF" "GBPJPY" "GBPNZD" "GBPUSD" "NAS100"
    "NZDJPY" "NZDUSD" "US500" "USDCAD" "USDCHF" "USDJPY"
)

# --- Script Logic ---
echo "Starting deletion process for non-_SB symbols..."
echo "Target InfluxDB Org: $ORG"
echo "Target Bucket: $BUCKET"
echo "Stop Time for Deletion Range: $STOP_TIME"
echo "---"

# Ensure Influx CLI is installed and in PATH
if ! command -v influx &> /dev/null
then
    echo "Error: influx CLI command could not be found. Please ensure it is installed and in your PATH."
    exit 1
fi

for SYM in "${SYMBOLS_TO_DELETE[@]}"; do
    echo "Attempting to delete data for symbol: $SYM"

    # Construct the predicate using the confirmed working syntax
    PREDICATE="_measurement=\"zones\" AND symbol=\"$SYM\""

    # Execute the influx delete command
    influx delete \
      --bucket "$BUCKET" \
      --org "$ORG" \
      --token "$TOKEN" \
      --start "$START_TIME" \
      --stop "$STOP_TIME" \
      --predicate "$PREDICATE"

    # Check the exit code of the influx delete command
    # Exit code 0 usually means success
    if [ $? -eq 0 ]; then
        echo "Successfully processed delete command for symbol: $SYM (this does not guarantee rows were deleted, only that the command ran without CLI error)."
    else
        echo "Warning: influx delete command for symbol $SYM exited with an error (code: $?). Please check any output above."
    fi
    echo "---"
    # Optional: Add a small delay between commands if you are concerned about server load,
    # though for 21 symbols it's likely not necessary.
    # sleep 0.5
done

echo "Deletion script finished processing all listed symbols."
echo "Please verify in InfluxDB that the data has been removed as expected."