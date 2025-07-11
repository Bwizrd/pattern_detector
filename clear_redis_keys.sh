#!/bin/bash

# Define container name
CONTAINER_NAME="trading_redis"

# Function to delete keys matching a pattern
delete_keys() {
  local pattern=$1
  echo "Deleting keys with pattern: $pattern"
  docker exec "$CONTAINER_NAME" redis-cli --scan --pattern "$pattern" | while read -r key; do
    docker exec "$CONTAINER_NAME" redis-cli DEL "$key"
  done
}

# Run for each pattern
delete_keys "pending_order:*"
delete_keys "order_lookup:*"
delete_keys "position_lookup:*"

echo "Done."