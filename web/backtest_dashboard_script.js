// Initialize dates to today 00:00 to tonight 23:59
function initializeDates() {
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const tonight = new Date(today.getTime() + 24 * 60 * 60 * 1000 - 1);
    
    // Format for datetime-local input
    const startStr = today.toISOString().slice(0, 16);
    const endStr = tonight.toISOString().slice(0, 16);
    
    document.getElementById('startTime').value = startStr;
    document.getElementById('endTime').value = endStr;
}

function getSelectedValues(name) {
    const checkboxes = document.querySelectorAll(`input[type="checkbox"][id*="${name}"]:checked`);
    return Array.from(checkboxes).map(cb => cb.value);
}

function createChart(containerId, data, title, valueKey = 'value') {
    const container = document.getElementById(containerId);
    if (!data || Object.keys(data).length === 0) {
        container.innerHTML = '<p>No data available</p>';
        return;
    }
    
    let html = '<div style="display: flex; flex-direction: column; gap: 8px;">';
    
    const maxValue = Math.max(...Object.values(data));
    
    for (const [key, value] of Object.entries(data)) {
        const percentage = maxValue > 0 ? (value / maxValue) * 100 : 0;
        const color = value >= 0 ? '#4ade80' : '#f87171';
        
        html += `
            <div style="display: flex; align-items: center; gap: 10px;">
                <div style="min-width: 80px; font-size: 0.9rem;">${key}</div>
                <div style="flex: 1; background: rgba(255,255,255,0.1); border-radius: 10px; height: 20px; position: relative;">
                    <div style="background: ${color}; width: ${percentage}%; height: 100%; border-radius: 10px; transition: all 0.3s ease;"></div>
                </div>
                <div style="min-width: 60px; text-align: right; color: ${color}; font-weight: bold;">${typeof value === 'number' ? value.toFixed(1) : value}</div>
            </div>
        `;
    }
    
    html += '</div>';
    container.innerHTML = html;
}

function displayResults(data) {
    const results = document.getElementById('results');
    results.style.display = 'block';
    
    // Update summary cards
    const summary = data.overall_summary;
    document.getElementById('totalTrades').textContent = summary.total_trades || 0;
    document.getElementById('totalTrades').className = 'value';
    
    const pnlElement = document.getElementById('totalPnl');
    pnlElement.textContent = summary.total_pnl_pips ? summary.total_pnl_pips.toFixed(1) : '0.0';
    pnlElement.className = `value ${summary.total_pnl_pips >= 0 ? 'positive' : 'negative'}`;
    
    const winRateElement = document.getElementById('winRate');
    winRateElement.textContent = summary.overall_win_rate_percent ? `${summary.overall_win_rate_percent.toFixed(1)}%` : '0%';
    winRateElement.className = `value ${summary.overall_win_rate_percent >= 50 ? 'positive' : 'negative'}`;
    
    document.getElementById('avgDuration').textContent = summary.average_trade_duration_str || 'N/A';
    document.getElementById('avgDuration').className = 'value';
    
    // Create charts
    createChart('symbolChart', summary.pnl_by_symbol_pips || summary.trades_by_symbol_percent, 'Symbol Performance');
    createChart('dayChart', summary.pnl_by_day_of_week_pips, 'Daily Performance');
    createChart('hourChart', summary.pnl_by_hour_of_day_pips, 'Hourly Performance');
    
    // Display detailed results
    const detailedContainer = document.getElementById('detailedResults');
    if (data.detailed_summaries && data.detailed_summaries.length > 0) {
        let detailedHtml = '<div style="display: flex; flex-direction: column; gap: 10px;">';
        
        data.detailed_summaries.forEach(detail => {
            const pnlColor = detail.total_pnl_pips >= 0 ? '#4ade80' : '#f87171';
            detailedHtml += `
                <div style="background: rgba(255,255,255,0.05); padding: 12px; border-radius: 8px; border-left: 4px solid ${pnlColor};">
                    <div style="font-weight: bold; margin-bottom: 5px;">${detail.symbol} (${detail.timeframe})</div>
                    <div style="display: flex; justify-content: space-between; font-size: 0.9rem;">
                        <span>Trades: ${detail.total_trades}</span>
                        <span>Win Rate: ${detail.win_rate_percent.toFixed(1)}%</span>
                        <span style="color: ${pnlColor};">P&L: ${detail.total_pnl_pips.toFixed(1)} pips</span>
                    </div>
                </div>
            `;
        });
        
        detailedHtml += '</div>';
        detailedContainer.innerHTML = detailedHtml;
    } else {
        detailedContainer.innerHTML = '<p>No detailed results available</p>';
    }
    
    // Populate trades table
    const tableBody = document.querySelector('#tradesTable tbody');
    tableBody.innerHTML = '';
    
    if (data.all_trades && data.all_trades.length > 0) {
        data.all_trades.forEach(trade => {
            const row = tableBody.insertRow();
            const pnlColor = trade.pnl_pips >= 0 ? '#4ade80' : '#f87171';
            
            row.innerHTML = `
                <td>${trade.symbol.replace('_SB', '')}</td>
                <td>${trade.direction}</td>
                <td>${new Date(trade.entry_time).toLocaleString()}</td>
                <td>${trade.entry_price.toFixed(5)}</td>
                <td>${new Date(trade.exit_time).toLocaleString()}</td>
                <td>${trade.exit_price.toFixed(5)}</td>
                <td style="color: ${pnlColor}; font-weight: bold;">${trade.pnl_pips.toFixed(1)}</td>
                <td>${trade.exit_reason}</td>
            `;
        });
    } else {
        const row = tableBody.insertRow();
        row.innerHTML = '<td colspan="8" style="text-align: center; color: #ccc;">No trades found</td>';
    }
}

async function runBacktest() {
    const button = document.querySelector('.run-button');
    const results = document.getElementById('results');
    
    // Show loading state
    button.innerHTML = '<div class="loading">Running Backtest...</div>';
    button.disabled = true;
    results.innerHTML = '<div class="loading">Processing backtest...</div>';
    results.style.display = 'block';
    
    try {
        // Collect form data
        const startTime = document.getElementById('startTime').value + ':00Z';
        const endTime = document.getElementById('endTime').value + ':59Z';
        const stopLossPips = parseFloat(document.getElementById('stopLoss').value);
        const takeProfitPips = parseFloat(document.getElementById('takeProfit').value);
        const lotSize = parseFloat(document.getElementById('lotSize').value);
        
        const patternTimeframes = getSelectedValues('tf');
        const allowedTradeDays = getSelectedValues('day');
        const symbols = getSelectedValues('sym');
        
        if (symbols.length === 0) {
            throw new Error('Please select at least one currency pair');
        }
        
        if (patternTimeframes.length === 0) {
            throw new Error('Please select at least one timeframe');
        }
        
        const requestBody = {
            start_time: startTime,
            end_time: endTime,
            symbols: symbols,
            pattern_timeframes: patternTimeframes,
            stop_loss_pips: stopLossPips,
            take_profit_pips: takeProfitPips,
            lot_size: lotSize,
            allowed_trade_days: allowedTradeDays
        };
        
        // Make API call with AbortController for proper cleanup
        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 300000); // 5 minute timeout
        
        try {
            const response = await fetch('http://localhost:8080/multi-symbol-backtest', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify(requestBody),
                signal: controller.signal
            });
            
            clearTimeout(timeoutId);
        
            if (!response.ok) {
                throw new Error(`HTTP error! status: ${response.status}`);
            }
            
            const data = await response.json();
        
        // Store results for export
        window.lastBacktestResults = data;
        
        // Reset results div structure
        results.innerHTML = `
            <div class="summary-cards">
                <div class="summary-card">
                    <h3>Total Trades</h3>
                    <div class="value" id="totalTrades">-</div>
                    <div class="label">Executed</div>
                </div>
                <div class="summary-card">
                    <h3>Total P&L</h3>
                    <div class="value" id="totalPnl">-</div>
                    <div class="label">Pips</div>
                </div>
                <div class="summary-card">
                    <h3>Win Rate</h3>
                    <div class="value" id="winRate">-</div>
                    <div class="label">Percentage</div>
                </div>
                <div class="summary-card">
                    <h3>Avg Duration</h3>
                    <div class="value" id="avgDuration">-</div>
                    <div class="label">Per Trade</div>
                </div>
            </div>
            
            <div class="charts-grid">
                <div class="chart-card">
                    <h3>📊 Performance by Symbol</h3>
                    <div id="symbolChart"></div>
                </div>
                <div class="chart-card">
                    <h3>📅 Performance by Day</h3>
                    <div id="dayChart"></div>
                </div>
                <div class="chart-card">
                    <h3>🕐 Performance by Hour</h3>
                    <div id="hourChart"></div>
                </div>
                <div class="chart-card">
                    <h3>📈 Detailed Results</h3>
                    <div id="detailedResults"></div>
                </div>
            </div>
            
            <div class="trades-table">
                <h3>📋 All Trades</h3>
                <div style="overflow-x: auto;">
                    <table id="tradesTable">
                        <thead>
                            <tr>
                                <th>Symbol</th>
                                <th>Direction</th>
                                <th>Entry Time</th>
                                <th>Entry Price</th>
                                <th>Exit Time</th>
                                <th>Exit Price</th>
                                <th>P&L (pips)</th>
                                <th>Exit Reason</th>
                            </tr>
                        </thead>
                        <tbody>
                        </tbody>
                    </table>
                </div>
            </div>
        `;
        
        displayResults(data);
        
        } catch (innerError) {
            clearTimeout(timeoutId);
            throw innerError;
        }
        
    } catch (error) {
        results.innerHTML = `
            <div class="error">
                ❌ <strong>Error:</strong> ${error.message}
                <br><small>Please check your backend server is running on localhost:8080</small>
            </div>
        `;
        console.error('Backtest error:', error);
    } finally {
        // Reset button
        button.innerHTML = '🚀 Run Backtest';
        button.disabled = false;
    }
}

// Keyboard shortcuts
document.addEventListener('keydown', function(e) {
    if (e.ctrlKey && e.key === 'Enter') {
        e.preventDefault();
        runBacktest();
    }
});

// Initialize dates when page loads
document.addEventListener('DOMContentLoaded', function() {
    initializeDates();
    
    // Add tooltips and enhanced UI interactions
    const cards = document.querySelectorAll('.summary-card, .chart-card, .checkbox-item');
    cards.forEach(card => {
        card.addEventListener('mouseenter', function() {
            this.style.transform = 'translateY(-2px)';
            this.style.boxShadow = '0 8px 25px rgba(0,0,0,0.15)';
        });
        
        card.addEventListener('mouseleave', function() {
            this.style.transform = 'translateY(0)';
            this.style.boxShadow = 'none';
        });
    });
    
    // Add click handlers for checkbox groups
    document.querySelectorAll('.checkbox-item').forEach(item => {
        item.addEventListener('click', function(e) {
            if (e.target.type !== 'checkbox') {
                const checkbox = this.querySelector('input[type="checkbox"]');
                checkbox.checked = !checkbox.checked;
            }
        });
    });
    
    // Add select all/none buttons for symbols
    const symbolsContainer = document.querySelector('.symbols-grid').parentElement;
    const symbolButtonsDiv = document.createElement('div');
    symbolButtonsDiv.style.cssText = 'margin-top: 10px; display: flex; gap: 10px; flex-wrap: wrap;';
    symbolButtonsDiv.innerHTML = `
        <button onclick="toggleAllSymbols(true)" style="padding: 8px 16px; background: rgba(255,255,255,0.1); border: 1px solid rgba(255,255,255,0.2); color: white; border-radius: 6px; cursor: pointer; transition: all 0.3s ease;">Select All</button>
        <button onclick="toggleAllSymbols(false)" style="padding: 8px 16px; background: rgba(255,255,255,0.1); border: 1px solid rgba(255,255,255,0.2); color: white; border-radius: 6px; cursor: pointer; transition: all 0.3s ease;">Select None</button>
        <button onclick="toggleMajorPairs()" style="padding: 8px 16px; background: rgba(255,255,255,0.1); border: 1px solid rgba(255,255,255,0.2); color: white; border-radius: 6px; cursor: pointer; transition: all 0.3s ease;">Major Pairs Only</button>
        <button onclick="exportResults('csv')" style="padding: 8px 16px; background: rgba(34, 197, 94, 0.2); border: 1px solid rgba(34, 197, 94, 0.4); color: #22c55e; border-radius: 6px; cursor: pointer; transition: all 0.3s ease;">📊 Export CSV</button>
    `;
    symbolsContainer.appendChild(symbolButtonsDiv);
});

function toggleAllSymbols(select) {
    const checkboxes = document.querySelectorAll('input[type="checkbox"][id*="sym"]');
    checkboxes.forEach(cb => cb.checked = select);
}

function toggleMajorPairs() {
    // First deselect all
    toggleAllSymbols(false);
    
    // Then select major pairs
    const majorPairs = ['symEURUSD', 'symGBPUSD', 'symUSDCHF', 'symAUDUSD', 'symUSDCAD', 'symNZDUSD', 'symUSDJPY'];
    majorPairs.forEach(id => {
        const checkbox = document.getElementById(id);
        if (checkbox) checkbox.checked = true;
    });
}

// Add export functionality
function exportResults(format = 'csv') {
    const data = window.lastBacktestResults;
    if (!data || !data.all_trades) {
        alert('No results to export');
        return;
    }
    
    if (format === 'csv') {
        let csv = 'Symbol,Timeframe,Direction,Entry Time,Entry Price,Exit Time,Exit Price,PnL Pips,Exit Reason,Day of Week,Hour\n';
        
        data.all_trades.forEach(trade => {
            csv += `${trade.symbol.replace('_SB', '')},${trade.timeframe},${trade.direction},"${trade.entry_time}",${trade.entry_price},"${trade.exit_time}",${trade.exit_price},${trade.pnl_pips},"${trade.exit_reason}",${trade.entry_day_of_week},${trade.entry_hour_of_day}\n`;
        });
        
        const blob = new Blob([csv], { type: 'text/csv' });
        const url = window.URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `backtest_results_${new Date().toISOString().split('T')[0]}.csv`;
        a.click();
        window.URL.revokeObjectURL(url);
    }
}