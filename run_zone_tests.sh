#!/bin/bash

# run_zone_tests.sh
# Script to run zone consistency tests from the tests/ folder

echo "🧪 Zone Consistency Test Runner"
echo "================================="
echo "Testing that endpoint and generator produce identical results"
echo ""

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: Please run this script from your project root directory"
    exit 1
fi

# Check if test file exists
if [ ! -f "tests/zone_consistency_tests.rs" ]; then
    echo "❌ Error: tests/zone_consistency_tests.rs not found"
    echo "Please ensure the test file is in your tests/ folder"
    exit 1
fi

# Check if .env file exists
if [ ! -f ".env" ]; then
    echo "⚠️  Warning: .env file not found. Creating template..."
    cat > .env << EOF
# InfluxDB Configuration for Testing
INFLUXDB_HOST=http://localhost:8086
INFLUXDB_ORG=your_org
INFLUXDB_TOKEN=your_token
INFLUXDB_BUCKET=your_bucket
GENERATOR_WRITE_BUCKET=your_write_bucket
GENERATOR_ZONE_MEASUREMENT=zones

# Logging
RUST_LOG=info
EOF
    echo "📝 Please edit .env with your actual InfluxDB settings"
    echo "   Then run this script again"
    exit 1
fi

# Source environment variables
set -a
source .env
set +a

echo "🔧 Environment loaded from .env"
echo "📊 Running zone consistency tests..."

# Function to run a test with nice output
run_test() {
    local test_name="$1"
    local description="$2"
    
    echo ""
    echo "▶️  $description"
    echo "   Running: $test_name"
    
    if cargo test "$test_name" -- --nocapture --test-threads=1; then
        echo "   ✅ PASSED"
        return 0
    else
        echo "   ❌ FAILED"
        return 1
    fi
}

# Track test results
passed_tests=0
total_tests=0

# Run individual tests
echo ""
echo "🔍 Starting test suite..."

# Test 1: Basic property tests
total_tests=$((total_tests + 1))
if run_test "test_zone_properties" "1️⃣  Testing zone property calculations"; then
    passed_tests=$((passed_tests + 1))
fi

# Test 2: Zone ID generation
total_tests=$((total_tests + 1))
if run_test "test_zone_id_generation" "2️⃣  Testing zone ID generation"; then
    passed_tests=$((passed_tests + 1))
fi

# Test 3: Deterministic behavior
total_tests=$((total_tests + 1))
if run_test "test_zone_id_deterministic" "3️⃣  Testing deterministic behavior"; then
    passed_tests=$((passed_tests + 1))
fi

# Test 4: Main comparison test
total_tests=$((total_tests + 1))
echo ""
echo "🎯 MAIN TEST: Comparing endpoint vs generator..."
if run_test "test_endpoint_vs_generator_same_zones" "4️⃣  Endpoint vs Generator comparison"; then
    passed_tests=$((passed_tests + 1))
    echo ""
    echo "🎉 CRITICAL TEST PASSED!"
    echo "   Your endpoint and generator produce identical results!"
else
    echo ""
    echo "💥 CRITICAL TEST FAILED!"
    echo "   Your endpoint and generator produce DIFFERENT results!"
    echo "   This indicates a consistency issue that needs investigation."
fi

# Test 5: Multi-symbol tests
total_tests=$((total_tests + 1))
if run_test "test_multiple_symbols_timeframes" "5️⃣  Testing multiple symbols and timeframes"; then
    passed_tests=$((passed_tests + 1))
fi

# Test Summary
echo ""
echo "📊 TEST SUMMARY"
echo "==============="
echo "Passed: $passed_tests/$total_tests tests"

if [ $passed_tests -eq $total_tests ]; then
    echo "🎉 ALL TESTS PASSED!"
    echo ""
    echo "✅ Your endpoint and generator are consistent!"
    echo "   Both code paths produce identical zone data."
    echo ""
    echo "🔒 Code Integrity Verified:"
    echo "   ✓ Same zone detection logic"
    echo "   ✓ Same activity calculations" 
    echo "   ✓ Same touch counting"
    echo "   ✓ Same zone IDs"
    echo "   ✓ Deterministic behavior"
    exit_code=0
else
    echo "❌ SOME TESTS FAILED!"
    echo ""
    echo "🚨 Consistency Issues Detected:"
    echo "   Your endpoint and generator may be producing different results."
    echo "   Review the test output above for specific differences."
    echo ""
    echo "🔧 Next Steps:"
    echo "   1. Check the failed test details above"
    echo "   2. Look for differences in zone processing logic"
    echo "   3. Verify environment configuration"
    echo "   4. Run individual tests for debugging:"
    echo "      cargo test test_endpoint_vs_generator_same_zones -- --nocapture"
    exit_code=1
fi

echo ""
echo "🧪 Additional Testing Options:"
echo ""
echo "Run specific tests:"
echo "  cargo test test_endpoint_vs_generator_same_zones -- --nocapture"
echo "  cargo test test_zone_id_deterministic -- --nocapture"
echo "  cargo test test_multiple_symbols_timeframes -- --nocapture"
echo ""
echo "Run with real InfluxDB data:"
echo "  cargo test test_with_real_influxdb_data --ignored -- --nocapture"
echo ""
echo "Run all zone tests:"
echo "  cargo test zone_consistency_tests -- --nocapture"
echo ""
echo "Debug with verbose output:"
echo "  RUST_LOG=debug cargo test test_endpoint_vs_generator_same_zones -- --nocapture"

exit $exit_code