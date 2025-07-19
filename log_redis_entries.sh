#!/bin/bash

# Define container name
CONTAINER_NAME="trading_redis"

# Function to log keys and their values matching a pattern
log_keys() {
  local pattern=$1
  echo "======================================="
  echo "Keys with pattern: $pattern"
  echo "======================================="
  
  local count=0
  docker exec "$CONTAINER_NAME" redis-cli --scan --pattern "$pattern" | while read -r key; do
    if [ -n "$key" ]; then
      count=$((count + 1))
      echo ""
      echo "[$count] Key: $key"
      echo "----------------------------------------"
      
      # Get the value and format it nicely
      value=$(docker exec "$CONTAINER_NAME" redis-cli GET "$key")
      
      if [ -n "$value" ]; then
        # Try to pretty-print JSON if it's valid JSON
        echo "$value" | python3 -m json.tool 2>/dev/null || echo "$value"
      else
        echo "(empty or null value)"
      fi
      echo "----------------------------------------"
    fi
  done
  
  # Count total keys
  total=$(docker exec "$CONTAINER_NAME" redis-cli --scan --pattern "$pattern" | wc -l)
  echo ""
  echo "Total keys found: $total"
  echo ""
}

# Function to get key count only
count_keys() {
  local pattern=$1
  local count=$(docker exec "$CONTAINER_NAME" redis-cli --scan --pattern "$pattern" | wc -l)
  echo "$pattern: $count keys"
}

echo "Redis Key Analysis - $(date)"
echo "Container: $CONTAINER_NAME"
echo ""

# First, show counts for all patterns
echo "=== KEY COUNTS ==="
count_keys "pending_order:*"
count_keys "order_lookup:*"
count_keys "position_lookup:*"
echo ""

# Then show detailed contents
log_keys "pending_order:*"
log_keys "order_lookup:*"
log_keys "position_lookup:*"

echo "======================================="
echo "Analysis complete - $(date)"
echo "======================================="