# Debug the actual data structure being returned

echo "🔍 Debugging Zone Data Structure"
echo "================================"

echo "1. Raw API response structure:"
curl -s "http://localhost:8080/ui/active-zones" | jq '.' | head -50

echo -e "\n2. First zone structure:"
curl -s "http://localhost:8080/ui/active-zones" | jq '.tradeable_zones[0] // .zones[0] // .[0] // {}'

echo -e "\n3. Available time fields in first zone:"
curl -s "http://localhost:8080/ui/active-zones" | jq '.tradeable_zones[0] // .zones[0] // .[0] // {}' | grep -E "(time|Time)" || echo "No time fields found"

echo -e "\n4. All field names in first zone:"
curl -s "http://localhost:8080/ui/active-zones" | jq '.tradeable_zones[0] // .zones[0] // .[0] // {} | keys'