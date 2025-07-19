#!/bin/bash

# Define container name
CONTAINER_NAME="trading_redis"

echo "=== PENDING ORDERS DEBUG ==="
echo "Container: $CONTAINER_NAME"
echo "Date: $(date)"
echo ""

# Count pending orders
count=$(docker exec "$CONTAINER_NAME" redis-cli --scan --pattern "pending_order:*" | wc -l)
echo "Total pending orders in Redis: $count"
echo ""

if [ $count -gt 0 ]; then
    echo "=== PENDING ORDER DETAILS ==="
    
    counter=1
    docker exec "$CONTAINER_NAME" redis-cli --scan --pattern "pending_order:*" | while read -r key; do
        if [ -n "$key" ]; then
            zone_id=${key#pending_order:}
            echo ""
            echo "[$counter] Zone ID: $zone_id"
            echo "    Redis Key: $key"
            
            # Get the order data
            order_data=$(docker exec "$CONTAINER_NAME" redis-cli GET "$key")
            
            if [ -n "$order_data" ]; then
                # Extract key fields from JSON
                symbol=$(echo "$order_data" | grep -o '"symbol":"[^"]*"' | cut -d'"' -f4)
                status=$(echo "$order_data" | grep -o '"status":"[^"]*"' | cut -d'"' -f4)
                order_type=$(echo "$order_data" | grep -o '"order_type":"[^"]*"' | cut -d'"' -f4)
                timeframe=$(echo "$order_data" | grep -o '"timeframe":"[^"]*"' | cut -d'"' -f4)
                
                echo "    Symbol: $symbol"
                echo "    Timeframe: $timeframe"
                echo "    Status: $status"
                echo "    Order Type: $order_type"
                
                # Show first 200 chars of full JSON for debugging
                echo "    Full JSON (truncated): ${order_data:0:200}..."
            else
                echo "    (No data found)"
            fi
            
            counter=$((counter + 1))
        fi
    done
else
    echo "No pending orders found in Redis."
fi

echo ""
echo "=== DEBUG COMPLETE ==="