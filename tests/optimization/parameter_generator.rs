// tests/optimization/parameter_generator.rs

// Generate a grid of parameter combinations
pub fn generate_parameter_grid(
    tp_range: &(f64, f64),
    sl_range: &(f64, f64),
    max_points: usize
) -> Vec<(f64, f64)> {
    let mut results = Vec::new();
    
    // Calculate how many points to sample in each dimension
    let dim_size = (max_points as f64).sqrt().ceil() as usize;
    
    // Generate TP points
    let mut tp_values = Vec::new();
    for i in 0..dim_size {
        let tp_value = tp_range.0 + (tp_range.1 - tp_range.0) * (i as f64 / (dim_size - 1) as f64);
        tp_values.push((tp_value * 10.0).round() / 10.0);
    }
    
    // Generate SL points
    let mut sl_values = Vec::new();
    for i in 0..dim_size {
        let sl_value = sl_range.0 + (sl_range.1 - sl_range.0) * (i as f64 / (dim_size - 1) as f64);
        sl_values.push((sl_value * 10.0).round() / 10.0);
    }
    
    // Add some specific values that are frequently good
    tp_values.push(5.0);
    tp_values.push(10.0);
    tp_values.push(15.0);
    tp_values.push(20.0);
    
    sl_values.push(2.0);
    sl_values.push(5.0);
    sl_values.push(10.0);
    
    // Generate combinations
    for &tp in &tp_values {
        for &sl in &sl_values {
            // Only include reasonable risk:reward ratios
            if tp > sl {
                results.push((tp, sl));
            }
        }
    }
    
    // Remove duplicates
    results.sort_by(|a, b| {
        let cmp = a.0.partial_cmp(&b.0).unwrap();
        if cmp == std::cmp::Ordering::Equal {
            a.1.partial_cmp(&b.1).unwrap()
        } else {
            cmp
        }
    });
    results.dedup();
    
    results
}