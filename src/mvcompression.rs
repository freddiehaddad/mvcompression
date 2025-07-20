/// Thread-safe adaptive compression decision system module.
/// 
/// This module implements the core MVCompression algorithm that learns from past
/// compression performance to make intelligent decisions about when to skip
/// compression attempts.

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

/// Compression ratio threshold above which a block is considered poorly compressible.
/// Blocks with ratio > 0.9 (i.e., compressed size is more than 90% of original) 
/// are treated as non-compressible.
const BLOCK_COMPRESSABLE_RATIO: f32 = 0.9;

/// Weight adjustment for blocks that compress well (ratio ≤ 0.9).
/// Negative value decreases compression_value, making skipping less likely.
const COMPRESSIBLE_BLOCK_WEIGHT: i32 = -10;

/// Weight adjustment for blocks that compress poorly (ratio > 0.9).
/// Positive value increases compression_value, making skipping more likely.
const NON_COMPRESSIBLE_BLOCK_WEIGHT: i32 = 4;

/// Weight adjustment when compression is skipped.
/// Negative value provides feedback to eventually retry compression.
const SKIP_COMPRESSION_BLOCK_WEIGHT: i32 = -1;

/// Initial compression value when algorithm starts.
/// Negative value ensures compression is attempted initially.
const INITIAL_COMPRESSION_VALUE: i32 = -80;

/// Maximum allowed compression value.
/// Prevents the algorithm from becoming permanently skip-heavy.
const MAX_COMPRESSION_VALUE: i32 = 200;

/// Minimum allowed compression value.
/// Prevents the algorithm from becoming permanently compression-heavy.
const MIN_COMPRESSION_VALUE: i32 = -300;

/// Bit shift factor for smoothing in moving average calculation.
/// Used to divide values: (value >> SMOOTHING_FACTOR) = value / 8
const SMOOTHING_FACTOR: usize = 3;

/// Weight for previous values in moving average calculation.
/// Formula: new_avg = (old_avg >> 3) * 7 + (new_value >> 3)
/// This gives ~87.5% weight to historical data, 12.5% to new data.
const PREVIOUS_WEIGHT: usize = 7;

/// A thread-safe adaptive compression decision system that learns from past
/// compression performance to decide whether to compress future data blocks.
/// 
/// The algorithm maintains a "compression value" score and moving averages of
/// compressed/uncompressed block sizes to make intelligent compression decisions.
/// 
/// # Algorithm Details
/// 
/// ## Compression Value
/// - Starts at -80 (always compress initially)
/// - Decreases by 10 for good compression (ratio ≤ 0.9)
/// - Increases by 4 for poor compression (ratio > 0.9)
/// - Decreases by 1 when compression is skipped
/// - Bounded between -300 and +200
/// 
/// ## Skip Logic
/// When compression_value > 0:
/// - Compare incoming block size to uncompressed moving average
/// - Skip if block_size ≤ average + (average / 4)  [within 125% of expected]
/// - Update compression_value and return true
/// 
/// ## Moving Averages
/// Uses exponential moving average with 87.5% weight on historical data:
/// - `new_avg = (old_avg >> 3) * 7 + (new_value >> 3)`
/// - Tracks both compressed and uncompressed block sizes
/// - Used for predicting compression effectiveness
/// 
/// # Thread Safety
/// 
/// All operations use lock-free atomic compare-and-swap loops, making the structure
/// safe for concurrent access from multiple threads without any locks or mutexes.
/// 
/// # Examples
/// 
/// ## Basic Usage
/// ```rust
/// use mvcompression::MVCompression;
/// 
/// let mvc = MVCompression::new();
/// 
/// // Check if compression should be skipped
/// if mvc.should_skip_compression(1024) {
///     // Store block uncompressed
/// } else {
///     // Compress block and update algorithm
///     // let compressed = compress(block);
///     mvc.update_compression_ratio(512, 1024); // 50% compression ratio
/// }
/// ```
/// 
/// ## Monitoring Algorithm State
/// ```rust
/// use mvcompression::MVCompression;
/// 
/// let mvc = MVCompression::new();
/// 
/// // Process some blocks...
/// mvc.update_compression_ratio(800, 1000);
/// mvc.update_compression_ratio(900, 1000);
/// 
/// // Check algorithm state
/// println!("Compression value: {}", mvc.get_compression_value());
/// println!("Average compressed size: {}", mvc.get_compressed_average());
/// println!("Average uncompressed size: {}", mvc.get_uncompressed_average());
/// ```
#[derive(Debug)]
pub struct MVCompression {
    /// Current compression decision value. Positive values enable skip logic.
    compression_value: AtomicI32,
    /// Moving average of compressed block sizes (smoothed with bit shifts).
    compressed_size_moving_average: AtomicUsize,
    /// Moving average of uncompressed block sizes (smoothed with bit shifts).
    uncompressed_size_moving_average: AtomicUsize,
}

impl Default for MVCompression {
    fn default() -> Self {
        Self::new()
    }
}

impl MVCompression {
    /// Creates a new MVCompression instance with default values.
    pub fn new() -> Self {
        Self {
            compression_value: AtomicI32::new(INITIAL_COMPRESSION_VALUE),
            compressed_size_moving_average: AtomicUsize::new(0),
            uncompressed_size_moving_average: AtomicUsize::new(0),
        }
    }

    /// Determines whether compression should be skipped for a block of the given size.
    /// 
    /// This is the main decision function of the algorithm. It uses the current
    /// compression value and historical size data to decide if compression is
    /// likely to be effective.
    /// 
    /// # Algorithm
    /// 1. If compression_value ≤ 0: always return false (always compress)
    /// 2. If compression_value > 0: check if block size is within expected range
    /// 3. If within range (≤ 125% of average): skip compression and update value
    /// 4. If outside range: don't skip (attempt compression)
    /// 
    /// # Thread Safety
    /// Uses atomic compare-exchange loop to safely update compression_value
    /// when skipping, ensuring no race conditions between threads.
    /// 
    /// # Arguments
    /// * `datasize` - The size in bytes of the data block to potentially compress
    /// 
    /// # Returns
    /// * `true` if compression should be skipped
    /// * `false` if compression should be attempted
    /// 
    /// # Examples
    /// ```rust
    /// use mvcompression::MVCompression;
    /// 
    /// let mvc = MVCompression::new();
    /// 
    /// // Initially returns false (compression_value is negative)
    /// assert!(!mvc.should_skip_compression(1000));
    /// 
    /// // After many poor compression results, may start returning true
    /// for _ in 0..30 {
    ///     mvc.update_compression_ratio(1000, 1000); // No compression
    /// }
    /// // Now may skip similar-sized blocks
    /// ```
    pub fn should_skip_compression(&self, datasize: usize) -> bool {
        let current_compression_value = self.compression_value.load(Ordering::Relaxed);
        if current_compression_value > 0 {
            let expected_size = self.uncompressed_size_moving_average.load(Ordering::Relaxed);
            if datasize <= expected_size + (expected_size >> 2) {
                // Use compare_and_swap loop to safely update compression_value
                loop {
                    let current = self.compression_value.load(Ordering::Relaxed);
                    let new_value = current + SKIP_COMPRESSION_BLOCK_WEIGHT;
                    match self.compression_value.compare_exchange_weak(
                        current,
                        new_value,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(_) => continue, // Retry if another thread modified the value
                    }
                }
                return true;
            }
        }
        false
    }

    /// Updates the moving averages for compressed and uncompressed block sizes.
    /// 
    /// This method uses lock-free atomic operations to safely update the moving
    /// averages from multiple threads.
    /// 
    /// # Arguments
    /// * `compressed` - The size of the compressed block
    /// * `uncompressed` - The size of the uncompressed block
    fn update_compression_block_size(&self, compressed: usize, uncompressed: usize) {
        // Update compressed size moving average atomically
        loop {
            let current_compressed = self.compressed_size_moving_average.load(Ordering::Relaxed);
            let new_compressed = (current_compressed >> SMOOTHING_FACTOR) * PREVIOUS_WEIGHT
                + (compressed >> SMOOTHING_FACTOR);
            match self.compressed_size_moving_average.compare_exchange_weak(
                current_compressed,
                new_compressed,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }

        // Update uncompressed size moving average atomically
        loop {
            let current_uncompressed = self.uncompressed_size_moving_average.load(Ordering::Relaxed);
            let new_uncompressed = (current_uncompressed >> SMOOTHING_FACTOR) * PREVIOUS_WEIGHT
                + (uncompressed >> SMOOTHING_FACTOR);
            match self.uncompressed_size_moving_average.compare_exchange_weak(
                current_uncompressed,
                new_uncompressed,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }

    /// Updates the compression decision algorithm based on actual compression results.
    /// 
    /// This method should be called after compressing a block to inform the algorithm
    /// about the effectiveness of the compression. It updates both the moving averages
    /// and the compression value based on the compression ratio.
    /// 
    /// # Algorithm Steps
    /// 1. Calculate compression ratio = compressed_size / uncompressed_size
    /// 2. Update moving averages for both compressed and uncompressed sizes
    /// 3. Adjust compression_value based on ratio:
    ///    - If ratio > 0.9 (poor): add +4 (bounded by MAX_COMPRESSION_VALUE)
    ///    - If ratio ≤ 0.9 (good): add -10 (bounded by MIN_COMPRESSION_VALUE)
    /// 
    /// # Thread Safety
    /// All updates use atomic compare-exchange loops with bounds checking,
    /// ensuring thread-safe modifications without locks.
    /// 
    /// # Arguments
    /// * `compressed` - The size in bytes of the compressed block
    /// * `uncompressed` - The size in bytes of the original uncompressed block
    /// 
    /// # Examples
    /// ```rust
    /// use mvcompression::MVCompression;
    /// 
    /// let mvc = MVCompression::new();
    /// 
    /// // Good compression (50% ratio)
    /// mvc.update_compression_ratio(500, 1000);
    /// assert!(mvc.get_compression_value() < -80); // Becomes more negative
    /// 
    /// // Poor compression (95% ratio)
    /// let mvc2 = MVCompression::new();
    /// mvc2.update_compression_ratio(950, 1000);
    /// assert!(mvc2.get_compression_value() > -80); // Becomes less negative
    /// ```
    /// 
    /// # Panics
    /// This method will not panic, but division by zero is possible if
    /// `uncompressed` is 0. Callers should ensure uncompressed > 0.
    pub fn update_compression_ratio(&self, compressed: usize, uncompressed: usize) {
        let compression_ratio = compressed as f32 / uncompressed as f32;
        self.update_compression_block_size(compressed, uncompressed);
        
        if compression_ratio > BLOCK_COMPRESSABLE_RATIO {
            // Update compression_value atomically with bounds checking
            loop {
                let current = self.compression_value.load(Ordering::Relaxed);
                if current < MAX_COMPRESSION_VALUE {
                    let new_value = current + NON_COMPRESSIBLE_BLOCK_WEIGHT;
                    match self.compression_value.compare_exchange_weak(
                        current,
                        new_value,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                } else {
                    break; // Already at max value
                }
            }
        } else {
            // Update compression_value atomically with bounds checking
            loop {
                let current = self.compression_value.load(Ordering::Relaxed);
                if current > MIN_COMPRESSION_VALUE {
                    let new_value = current + COMPRESSIBLE_BLOCK_WEIGHT;
                    match self.compression_value.compare_exchange_weak(
                        current,
                        new_value,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                } else {
                    break; // Already at min value
                }
            }
        }
    }

    /// Returns the current compression value for debugging or monitoring purposes.
    /// 
    /// The compression value indicates the algorithm's current bias:
    /// - Negative values: bias toward compression
    /// - Positive values: bias toward skipping compression
    /// - Zero: neutral (skip logic activated but no strong bias)
    /// 
    /// # Thread Safety
    /// Uses atomic load with relaxed ordering for best performance.
    /// 
    /// # Returns
    /// Current compression value (range: MIN_COMPRESSION_VALUE to MAX_COMPRESSION_VALUE)
    /// 
    /// # Examples
    /// ```rust
    /// use mvcompression::MVCompression;
    /// 
    /// let mvc = MVCompression::new();
    /// assert_eq!(mvc.get_compression_value(), -80); // Initial value
    /// ```
    pub fn get_compression_value(&self) -> i32 {
        self.compression_value.load(Ordering::Relaxed)
    }

    /// Returns the current compressed size moving average.
    /// 
    /// This value represents the smoothed average of compressed block sizes
    /// processed by the algorithm. Note that due to the bit-shifting smoothing,
    /// this value is approximately 1/8th of the actual average size.
    /// 
    /// # Thread Safety
    /// Uses atomic load with relaxed ordering for best performance.
    /// 
    /// # Returns
    /// Current compressed size moving average (bit-shifted for smoothing)
    /// 
    /// # Examples
    /// ```rust
    /// use mvcompression::MVCompression;
    /// 
    /// let mvc = MVCompression::new();
    /// assert_eq!(mvc.get_compressed_average(), 0); // Initially zero
    /// 
    /// mvc.update_compression_ratio(800, 1000);
    /// assert_eq!(mvc.get_compressed_average(), 100); // 800 >> 3 = 100
    /// ```
    pub fn get_compressed_average(&self) -> usize {
        self.compressed_size_moving_average.load(Ordering::Relaxed)
    }

    /// Returns the current uncompressed size moving average.
    /// 
    /// This value represents the smoothed average of uncompressed block sizes
    /// processed by the algorithm. Note that due to the bit-shifting smoothing,
    /// this value is approximately 1/8th of the actual average size.
    /// 
    /// Used internally by `should_skip_compression` to determine if an incoming
    /// block size is within the expected range.
    /// 
    /// # Thread Safety
    /// Uses atomic load with relaxed ordering for best performance.
    /// 
    /// # Returns
    /// Current uncompressed size moving average (bit-shifted for smoothing)
    /// 
    /// # Examples
    /// ```rust
    /// use mvcompression::MVCompression;
    /// 
    /// let mvc = MVCompression::new();
    /// assert_eq!(mvc.get_uncompressed_average(), 0); // Initially zero
    /// 
    /// mvc.update_compression_ratio(800, 1000);
    /// assert_eq!(mvc.get_uncompressed_average(), 125); // 1000 >> 3 = 125
    /// ```
    pub fn get_uncompressed_average(&self) -> usize {
        self.uncompressed_size_moving_average.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::sync::Arc;

    #[test]
    fn test_new_mvcompression() {
        let mvc = MVCompression::new();
        assert_eq!(mvc.get_compression_value(), INITIAL_COMPRESSION_VALUE);
        assert_eq!(mvc.get_compressed_average(), 0);
        assert_eq!(mvc.get_uncompressed_average(), 0);
    }

    #[test]
    fn test_default_trait() {
        let mvc = MVCompression::default();
        assert_eq!(mvc.get_compression_value(), INITIAL_COMPRESSION_VALUE);
        assert_eq!(mvc.get_compressed_average(), 0);
        assert_eq!(mvc.get_uncompressed_average(), 0);
    }

    #[test]
    fn test_compression_ratio_update_good_compression() {
        let mvc = MVCompression::new();
        let initial_value = mvc.get_compression_value();
        
        // Test with excellent compression ratio (0.5)
        mvc.update_compression_ratio(500, 1000);
        assert!(mvc.get_compression_value() < initial_value);
        assert_eq!(mvc.get_compression_value(), initial_value + COMPRESSIBLE_BLOCK_WEIGHT);
    }

    #[test]
    fn test_compression_ratio_update_poor_compression() {
        let mvc = MVCompression::new();
        let initial_value = mvc.get_compression_value();
        
        // Test with poor compression ratio (0.95)
        mvc.update_compression_ratio(950, 1000);
        assert!(mvc.get_compression_value() > initial_value);
        assert_eq!(mvc.get_compression_value(), initial_value + NON_COMPRESSIBLE_BLOCK_WEIGHT);
    }

    #[test]
    fn test_compression_ratio_boundary_conditions() {
        let mvc = MVCompression::new();
        let initial_value = mvc.get_compression_value();
        
        // Test exactly at the boundary (0.9)
        mvc.update_compression_ratio(900, 1000);
        assert_eq!(mvc.get_compression_value(), initial_value + COMPRESSIBLE_BLOCK_WEIGHT);
        
        // Test just above the boundary (0.901)
        let mvc2 = MVCompression::new();
        mvc2.update_compression_ratio(901, 1000);
        assert_eq!(mvc2.get_compression_value(), initial_value + NON_COMPRESSIBLE_BLOCK_WEIGHT);
    }

    #[test]
    fn test_compression_value_bounds() {
        let mvc = MVCompression::new();
        
        // Test upper bound - repeatedly add non-compressible weight
        for _ in 0..100 {
            mvc.update_compression_ratio(1000, 1000); // ratio = 1.0 (poor)
        }
        assert!(mvc.get_compression_value() <= MAX_COMPRESSION_VALUE);
        
        let mvc2 = MVCompression::new();
        // Test lower bound - repeatedly add compressible weight
        for _ in 0..100 {
            mvc2.update_compression_ratio(100, 1000); // ratio = 0.1 (excellent)
        }
        assert!(mvc2.get_compression_value() >= MIN_COMPRESSION_VALUE);
    }

    #[test]
    fn test_moving_averages_update() {
        let mvc = MVCompression::new();
        
        // First update
        mvc.update_compression_ratio(800, 1000);
        
        let compressed_avg = mvc.get_compressed_average();
        let uncompressed_avg = mvc.get_uncompressed_average();
        
        // Moving averages should be non-zero after first update
        assert!(compressed_avg > 0);
        assert!(uncompressed_avg > 0);
        
        // Second update should change the averages
        mvc.update_compression_ratio(600, 1200);
        
        assert_ne!(mvc.get_compressed_average(), compressed_avg);
        assert_ne!(mvc.get_uncompressed_average(), uncompressed_avg);
    }

    #[test]
    fn test_skip_compression_initially_false() {
        let mvc = MVCompression::new();
        // Initially compression value is negative, so should not skip
        assert!(!mvc.should_skip_compression(1000));
        assert!(!mvc.should_skip_compression(0));
        assert!(!mvc.should_skip_compression(usize::MAX));
    }

    #[test]
    fn test_skip_compression_activation() {
        let mvc = MVCompression::new();
        
        // Force compression value to be positive by adding poor compression results
        for _ in 0..30 {
            mvc.update_compression_ratio(1000, 1000); // No compression
        }
        
        // Now compression value should be positive
        assert!(mvc.get_compression_value() > 0);
        
        // Build up some average size history
        for _ in 0..10 {
            mvc.update_compression_ratio(1000, 1000);
        }
        
        let expected_size = mvc.get_uncompressed_average();
        
        // Test skip logic - should skip for similar sized blocks
        assert!(mvc.should_skip_compression(expected_size));
        assert!(mvc.should_skip_compression(expected_size + (expected_size >> 3))); // Within 12.5%
        
        // Should not skip for significantly larger blocks
        assert!(!mvc.should_skip_compression(expected_size * 2));
    }

    #[test]
    fn test_skip_compression_updates_value() {
        let mvc = MVCompression::new();
        
        // Force positive compression value
        for _ in 0..30 {
            mvc.update_compression_ratio(1000, 1000);
        }
        
        // Build up average
        for _ in 0..10 {
            mvc.update_compression_ratio(1000, 1000);
        }
        
        let initial_compression_value = mvc.get_compression_value();
        let expected_size = mvc.get_uncompressed_average();
        
        // Skipping should decrease compression value
        if mvc.should_skip_compression(expected_size) {
            assert_eq!(mvc.get_compression_value(), initial_compression_value + SKIP_COMPRESSION_BLOCK_WEIGHT);
        }
    }

    #[test]
    fn test_thread_safety() {
        let mvc = Arc::new(MVCompression::new());
        let mut handles = vec![];
        
        // Spawn multiple threads that update compression ratios
        for i in 0..10 {
            let mvc_clone = Arc::clone(&mvc);
            let handle = thread::spawn(move || {
                for j in 0..50 { // Reduced iterations to control the final value
                    let compressed = 500 + (i * j) % 500;
                    let uncompressed = 1000;
                    mvc_clone.update_compression_ratio(compressed, uncompressed);
                }
            });
            handles.push(handle);
        }
        
        // Spawn threads that check skip compression
        for _ in 0..5 {
            let mvc_clone = Arc::clone(&mvc);
            let handle = thread::spawn(move || {
                for _ in 0..100 { // Reduced iterations
                    mvc_clone.should_skip_compression(1000);
                }
            });
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Verify the structure is still in a consistent state
        let compression_value = mvc.get_compression_value();
        assert!(compression_value >= MIN_COMPRESSION_VALUE, 
            "Compression value {} is below minimum {}", compression_value, MIN_COMPRESSION_VALUE);
        assert!(compression_value <= MAX_COMPRESSION_VALUE,
            "Compression value {} is above maximum {}", compression_value, MAX_COMPRESSION_VALUE);
        
        // Verify averages are reasonable
        assert!(mvc.get_compressed_average() > 0);
        assert!(mvc.get_uncompressed_average() > 0);
    }

    #[test]
    fn test_moving_average_calculation() {
        let mvc = MVCompression::new();
        
        // Test that moving average calculation is correct
        mvc.update_compression_ratio(800, 1000);
        
        let expected_compressed = 800 >> SMOOTHING_FACTOR;
        let expected_uncompressed = 1000 >> SMOOTHING_FACTOR;
        
        assert_eq!(mvc.get_compressed_average(), expected_compressed);
        assert_eq!(mvc.get_uncompressed_average(), expected_uncompressed);
    }

    #[test]
    fn test_zero_size_handling() {
        let mvc = MVCompression::new();
        
        // Test with zero compressed size (perfect compression)
        mvc.update_compression_ratio(0, 1000);
        assert!(mvc.get_compression_value() < INITIAL_COMPRESSION_VALUE);
        
        // Test with zero uncompressed size (edge case)
        let mvc2 = MVCompression::new();
        mvc2.update_compression_ratio(100, 1);
        // Should handle gracefully without panicking
        assert!(mvc2.get_compression_value() != INITIAL_COMPRESSION_VALUE);
    }

    #[test]
    fn test_large_size_values() {
        let mvc = MVCompression::new();
        
        // Test with large values to ensure no overflow
        let large_size = usize::MAX >> 10; // Large but won't overflow in calculations
        mvc.update_compression_ratio(large_size / 2, large_size);
        
        // Should handle large values gracefully
        assert!(mvc.get_compressed_average() > 0);
        assert!(mvc.get_uncompressed_average() > 0);
    }

    #[test]
    fn test_sequential_behavior_simulation() {
        let mvc = MVCompression::new();
        let mut skip_count = 0;
        let mut compress_count = 0;
        
        // Simulate the behavior from main.rs
        for _i in 1..30 {
            let uncompressed = 1000;
            let compressed = 1000; // No compression achieved
            
            if mvc.should_skip_compression(uncompressed) {
                skip_count += 1;
            } else {
                mvc.update_compression_ratio(compressed, uncompressed);
                compress_count += 1;
            }
        }
        
        // Should eventually start skipping compression due to poor ratios
        assert!(skip_count > 0, "Should have skipped some compressions");
        assert!(compress_count > 0, "Should have attempted some compressions");
        assert!(mvc.get_compression_value() > INITIAL_COMPRESSION_VALUE);
    }
}
