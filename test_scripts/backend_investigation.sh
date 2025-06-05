# Backend Data Investigation Script
# This will help identify where the bad data is coming from

echo "🔍 Backend Data Investigation"
echo "============================"

echo "1. Checking different backend endpoints for data consistency..."

echo -e "\n📊 Testing /ui/active-zones endpoint:"
curl -s "http://localhost:8080/ui/active-zones" | jq '{
  total_zones: (.tradeable_zones | length),
  sample_zone: .tradeable_zones[0] | {
    zone_id, symbol, timeframe, zone_high, zone_low, is_active, 
    start_time, end_time, bars_active, start_idx, end_idx
  }
}'

echo -e "\n📊 Testing /bulk-multi-tf-active-zones endpoint:"
curl -s -X POST "http://localhost:8080/bulk-multi-tf-active-zones" \
  -H "Content-Type: application/json" \
  -d '[{"symbol": "USDCAD", "timeframes": ["30m"]}]' \
  -G -d "start_time=2025-05-01T00:00:00Z" -d "end_time=2025-05-30T00:00:00Z" -d "pattern=fifty_percent_before_big_bar" | \
jq '{
  total_results: (. | length),
  sample_result: .[0] | {
    symbol, timeframe, status,
    sample_zone: .data.supply_zones[0] // .data.demand_zones[0] | {
      zone_id, zone_high, zone_low, is_active, 
      start_time, end_time, bars_active, start_idx, end_idx
    }
  }
}' 2>/dev/null || echo "Bulk endpoint failed or no data"

echo -e "\n📊 Testing /latest-formed-zones endpoint:"
curl -s "http://localhost:8080/ui/latest-zones" | jq '{
  total_zones: (.zones | length),
  sample_zone: .zones[0] | {
    zone_id, symbol, timeframe, zone_high, zone_low, is_active, 
    start_time, end_time, bars_active, start_idx, end_idx
  }
}' 2>/dev/null || echo "Latest zones endpoint failed or no data"

echo -e "\n🔍 Data Quality Analysis:"

echo "Checking for zones with invalid zone_high values:"
INVALID_HIGH_COUNT=$(curl -s "http://localhost:8080/ui/active-zones" | jq '[.tradeable_zones[] | select(.zone_high <= 0)] | length')
echo "Zones with zone_high <= 0: $INVALID_HIGH_COUNT"

echo "Checking for inactive zones without end_time:"
INACTIVE_NO_END=$(curl -s "http://localhost:8080/ui/active-zones" | jq '[.tradeable_zones[] | select(.is_active == false and (.end_time == null or .end_time == ""))] | length')
echo "Inactive zones without end_time: $INACTIVE_NO_END"

echo "Checking for zones with null start_idx/end_idx:"
NULL_IDX_COUNT=$(curl -s "http://localhost:8080/ui/active-zones" | jq '[.tradeable_zones[] | select(.start_idx == null or .end_idx == null)] | length')
echo "Zones with null indices: $NULL_IDX_COUNT"

echo "Checking bars_active distribution:"
curl -s "http://localhost:8080/ui/active-zones" | jq '[.tradeable_zones[].bars_active] | {
  min: min,
  max: max,
  avg: (add / length | floor)
}'

echo -e "\n🗄️ Database Investigation:"
echo "This suggests we need to check:"
echo "1. Which InfluxDB measurement /ui/active-zones is querying"
echo "2. Whether it's using old data from before the generator fix"
echo "3. If there's a different enrichment process for this endpoint"
echo "4. Whether the endpoint needs to be updated to use the generator's data"