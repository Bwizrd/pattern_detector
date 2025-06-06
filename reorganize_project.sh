#!/bin/bash
# reorganize_project.sh - Reorganize Rust project into logical modules

set -e  # Exit on any error

echo "🗂️  Reorganizing Rust project structure..."
echo "=========================================="

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ] || [ ! -d "src" ]; then
    echo "❌ Error: Run this script from the project root (where Cargo.toml is located)"
    exit 1
fi

# Create backup
echo "📦 Creating backup..."
cp -r src src_backup_$(date +%Y%m%d_%H%M%S)
echo "✅ Backup created"

# Create new directory structure
echo "📁 Creating new directory structure..."
mkdir -p src/{api,cache,data,zones,trading/backtest,realtime,notifications}
echo "✅ Directories created"

# Create mod.rs files
echo "📝 Creating mod.rs files..."

# src/api/mod.rs
cat > src/api/mod.rs << 'EOF'
pub mod detect;
pub mod cache_endpoints;
pub mod get_zone_by_id;
pub mod latest_zones_handler;
pub mod multi_backtest_handler;
pub mod optimize_handler;
EOF

# src/cache/mod.rs
cat > src/cache/mod.rs << 'EOF'
pub mod minimal_zone_cache;
pub mod realtime_cache_updater;
EOF

# src/data/mod.rs
cat > src/data/mod.rs << 'EOF'
pub mod influx_fetcher;
pub mod ctrader_integration;
EOF

# src/zones/mod.rs
cat > src/zones/mod.rs << 'EOF'
pub mod zone_detection;
pub mod patterns;
EOF

# src/trading/mod.rs
cat > src/trading/mod.rs << 'EOF'
pub mod trade_decision_engine;
pub mod trade_event_logger;
pub mod trades;
pub mod trading;
pub mod backtest;
EOF

# src/trading/backtest/mod.rs
cat > src/trading/backtest/mod.rs << 'EOF'
pub mod backtest;
pub mod backtest_api;
EOF

# src/realtime/mod.rs
cat > src/realtime/mod.rs << 'EOF'
pub mod realtime_zone_monitor;
pub mod proximity_detector;
pub mod websocket_server;
pub mod simple_price_websocket;
EOF

# src/notifications/mod.rs
cat > src/notifications/mod.rs << 'EOF'
pub mod notification_manager;
pub mod telegram_notifier;
pub mod sound_notifier;
EOF

echo "✅ mod.rs files created"

# Move files to new locations
echo "🚚 Moving files to new locations..."

# Move API endpoints
echo "  📡 Moving API files..."
[ -f src/detect.rs ] && mv src/detect.rs src/api/
[ -f src/cache_endpoints.rs ] && mv src/cache_endpoints.rs src/api/
[ -f src/get_zone_by_id.rs ] && mv src/get_zone_by_id.rs src/api/
[ -f src/latest_zones_handler.rs ] && mv src/latest_zones_handler.rs src/api/
[ -f src/multi_backtest_handler.rs ] && mv src/multi_backtest_handler.rs src/api/
[ -f src/optimize_handler.rs ] && mv src/optimize_handler.rs src/api/

# Move cache files
echo "  💾 Moving cache files..."
[ -f src/minimal_zone_cache.rs ] && mv src/minimal_zone_cache.rs src/cache/
[ -f src/realtime_cache_updater.rs ] && mv src/realtime_cache_updater.rs src/cache/

# Move data fetching files
echo "  📊 Moving data files..."
[ -f src/influx_fetcher.rs ] && mv src/influx_fetcher.rs src/data/
[ -f src/ctrader_integration.rs ] && mv src/ctrader_integration.rs src/data/

# Move zone detection files
echo "  🎯 Moving zone detection files..."
[ -f src/zone_detection.rs ] && mv src/zone_detection.rs src/zones/
[ -d src/patterns ] && mv src/patterns src/zones/

# Move trading files
echo "  💰 Moving trading files..."
[ -f src/trade_decision_engine.rs ] && mv src/trade_decision_engine.rs src/trading/
[ -f src/trade_event_logger.rs ] && mv src/trade_event_logger.rs src/trading/
[ -f src/trades.rs ] && mv src/trades.rs src/trading/
[ -f src/trading.rs ] && mv src/trading.rs src/trading/
[ -f src/backtest.rs ] && mv src/backtest.rs src/trading/backtest/
[ -f src/backtest_api.rs ] && mv src/backtest_api.rs src/trading/backtest/

# Move real-time files
echo "  ⚡ Moving real-time files..."
[ -f src/realtime_zone_monitor.rs ] && mv src/realtime_zone_monitor.rs src/realtime/
[ -f src/proximity_detector.rs ] && mv src/proximity_detector.rs src/realtime/
[ -f src/websocket_server.rs ] && mv src/websocket_server.rs src/realtime/
[ -f src/simple_price_websocket.rs ] && mv src/simple_price_websocket.rs src/realtime/

# Move notification files
echo "  📢 Moving notification files..."
[ -f src/notification_manager.rs ] && mv src/notification_manager.rs src/notifications/
[ -f src/telegram_notifier.rs ] && mv src/telegram_notifier.rs src/notifications/
[ -f src/sound_notifier.rs ] && mv src/sound_notifier.rs src/notifications/

echo "✅ Files moved successfully"

# Update main.rs module declarations
echo "🔧 Updating main.rs module declarations..."

# Create a backup of main.rs
cp src/main.rs src/main.rs.backup

# Create new main.rs with updated module declarations
cat > src/main_modules_temp.rs << 'EOF'
// Module declarations - Updated for new structure
mod api;
mod cache;
mod data;
mod zones;
mod trading;
mod realtime;
mod notifications;

// Re-export commonly used modules for easier access
pub use api::*;
pub use cache::*;
pub use data::*;
pub use zones::*;
pub use trading::*;
pub use realtime::*;
pub use notifications::*;

// Keep existing types, errors, and other core modules at root level
mod types;
mod errors;
EOF

# Extract the rest of main.rs (after module declarations) and append
tail -n +20 src/main.rs.backup >> src/main_modules_temp.rs

# Replace main.rs
mv src/main_modules_temp.rs src/main.rs

echo "✅ main.rs updated"

# Show final structure
echo ""
echo "📋 New project structure:"
echo "========================="
tree src/ -I 'target|.git'

echo ""
echo "🎉 Reorganization complete!"
echo ""
echo "⚠️  IMPORTANT NEXT STEPS:"
echo "1. Update import paths in your files from:"
echo "   use crate::detect::..."
echo "   to:"
echo "   use crate::api::detect::..."
echo ""
echo "2. Test the project builds:"
echo "   cargo check"
echo "   cargo build"
echo ""
echo "3. If there are compilation errors, update the import paths in the affected files"
echo ""
echo "4. Backup created: src_backup_$(date +%Y%m%d_%H%M%S)/"
echo "   (You can delete this once everything works)"
echo ""
echo "🔍 Common import path changes needed:"
echo "   crate::minimal_zone_cache        → crate::cache::minimal_zone_cache"
echo "   crate::zone_detection            → crate::zones::zone_detection"
echo "   crate::detect                    → crate::api::detect"
echo "   crate::notification_manager      → crate::notifications::notification_manager"
echo "   crate::realtime_zone_monitor     → crate::realtime::realtime_zone_monitor"
echo "   crate::patterns::FiftyPercent... → crate::zones::patterns::FiftyPercent..."chmoddddd