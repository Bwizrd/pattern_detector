#!/bin/bash

# Script to extract Generator vs API RAW JSON outputs from logs for comparison (using awk flag - revised execution)

LOG_FILE="backend.log" # Make sure this points to your actual log file
OUTPUT_DIR="./json_comparison" # Directory to store the output JSON files

# Check if log file exists
if [ ! -f "$LOG_FILE" ]; then
    echo "Error: Log file '$LOG_FILE' not found!"
    exit 1
fi

# Create output directory if it doesn't exist
mkdir -p "$OUTPUT_DIR"

echo "Extracting RAW JSON outputs using awk..."
echo "Output files will be placed in: ${OUTPUT_DIR}"
echo "Use a diff tool to compare corresponding _generator.json and _api.json files."
echo "--------------------------------------------------"

# Define the awk script as a variable (using single quotes carefully)
# Use index() for finding the header to avoid regex issues
# Exit after the first block is fully processed
read -r -d '' AWK_SCRIPT <<'EOF_AWK'
BEGIN { printing=0; found=0 }
# Check if the header string starts at the beginning of the line
index($0, header) == 1 && found == 0 {
    printing=1; # Start printing from the next line
    found=1;    # Mark as found
    next;       # Skip the header line itself
}
# If printing flag is set
printing {
    print;      # Print the current line (part of JSON)
    # If this line contains ONLY the closing brace '}', stop printing
    if ($0 == "}") {
        printing=0;
        # Exit awk completely after finding the first full block
        # Some awk versions might need just 'exit' without status
        exit 0;
    }
}
EOF_AWK


# Use a 'here document' (<<EOF) to define the list of zones to check
# Format: Symbol Timeframe (space-separated - StartTime not needed for grep)
while read -r symbol timeframe _; do # Read symbol and timeframe, ignore start_time
    # Skip empty lines or lines starting with #
    [[ -z "$symbol" || "$symbol" == \#* ]] && continue

    # Construct the symbol variations needed for grep headers
    symbol_sb="${symbol}_SB"
    base_filename="${symbol}_${timeframe}" # e.g., AUDCAD_30m

    # Define output filenames
    generator_json_file="${OUTPUT_DIR}/${base_filename}_generator.json"
    api_json_file="${OUTPUT_DIR}/${base_filename}_api.json"

    echo "Processing: ${symbol}/${timeframe} ..."

    # --- Extract Generator JSON using awk ---
    # Define the specific header string pattern for awk variable
    # No regex needed here, just exact string matching with index()
    generator_header_pattern="[RAW_DETECTION_RESULTS] ${symbol_sb}/${timeframe}:"
    # Execute awk, passing the header and the script
    awk -v header="${generator_header_pattern}" "${AWK_SCRIPT}" "$LOG_FILE" > "$generator_json_file"

    # Check if the generator file was created and is not empty
    if [ ! -s "$generator_json_file" ]; then
        echo "  WARNING: Could not extract Generator JSON for ${symbol}/${timeframe}"
    else
         echo "  -> Created ${generator_json_file}"
    fi

    # --- Extract API JSON using awk ---
    # Define the specific header string pattern for awk variable
    api_header_pattern="[API_RAW_JSON] [Task ${symbol}/${timeframe}]"
    # Execute awk, passing the header and the script
    awk -v header="${api_header_pattern}" "${AWK_SCRIPT}" "$LOG_FILE" > "$api_json_file"

     # Check if the API file was created and is not empty
    if [ ! -s "$api_json_file" ]; then
        echo "  WARNING: Could not extract API JSON for ${symbol}/${timeframe}"
    else
        echo "  -> Created ${api_json_file}"
    fi
    echo ""

done <<EOF
# List of the 18 potentially missing zones (Symbol Timeframe StartTime - StartTime ignored now)
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

echo "Extraction finished. Please compare files in ${OUTPUT_DIR} using a diff tool."