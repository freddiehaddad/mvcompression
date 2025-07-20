//! # MVCompression - Adaptive Compression Decision System
//! 
//! A thread-safe, lock-free adaptive compression decision system that learns from past
//! compression performance to intelligently decide whether to compress future data blocks.
//! 
//! ## Overview
//! 
//! The MVCompression algorithm maintains a "compression value" score and moving averages of
//! compressed/uncompressed block sizes to make intelligent compression decisions. It adapts
//! its behavior based on historical compression effectiveness, automatically skipping
//! compression when it's likely to be ineffective.
//! 
//! ## Key Features
//! 
//! - **Thread-safe**: Uses lock-free atomic operations for concurrent access
//! - **Adaptive**: Learns from compression effectiveness over time
//! - **Efficient**: Minimizes unnecessary compression attempts
//! - **Self-tuning**: Automatically adjusts behavior based on data characteristics
//! - **Lock-free**: High-performance concurrent operations without blocking
//! 
//! ## How It Works
//! 
//! 1. **Compression Value**: Starts at -80 and adjusts based on compression results
//!    - Good compression (ratio ≤ 0.9): decreases value by 10
//!    - Poor compression (ratio > 0.9): increases value by 4
//!    - Skip events: decreases value by 1
//! 
//! 2. **Skip Logic**: When compression value becomes positive:
//!    - Compares incoming block size to historical average
//!    - Skips compression if size is within 25% of expected size
//! 
//! 3. **Moving Averages**: Tracks compressed and uncompressed block sizes
//!    - Uses exponential moving average with smoothing factor
//!    - Helps predict future compression effectiveness
//! 
//! ## Example Usage
//! 
//! ```rust,no_run
//! use mvcompression::MVCompression;
//! 
//! // Mock functions for demonstration
//! fn get_data_blocks() -> Vec<Vec<u8>> { vec![] }
//! fn store_uncompressed(_data: Vec<u8>) {}
//! fn compress(data: &[u8]) -> Vec<u8> { data.to_vec() }
//! fn store_compressed(_data: Vec<u8>) {}
//! 
//! let mvc = MVCompression::new();
//! 
//! // Process data blocks
//! for block_data in get_data_blocks() {
//!     if mvc.should_skip_compression(block_data.len()) {
//!         // Skip compression for this block
//!         store_uncompressed(block_data);
//!     } else {
//!         // Attempt compression
//!         let compressed = compress(&block_data);
//!         
//!         // Update algorithm with results
//!         mvc.update_compression_ratio(compressed.len(), block_data.len());
//!         store_compressed(compressed);
//!     }
//! }
//! ```
//! 
//! ## Thread Safety Example
//! 
//! ```rust
//! use mvcompression::MVCompression;
//! use std::sync::Arc;
//! use std::thread;
//! 
//! let mvc = Arc::new(MVCompression::new());
//! 
//! // Spawn multiple worker threads
//! let handles: Vec<_> = (0..4).map(|_| {
//!     let mvc = Arc::clone(&mvc);
//!     thread::spawn(move || {
//!         // Each thread can safely use the same MVCompression instance
//!         if mvc.should_skip_compression(1024) {
//!             // Skip compression
//!         } else {
//!             // Perform compression and update
//!             mvc.update_compression_ratio(512, 1024);
//!         }
//!     })
//! }).collect();
//! 
//! for handle in handles {
//!     handle.join().unwrap();
//! }
//! ```
//! 
//! ## Algorithm Parameters
//! 
//! The algorithm uses several tunable constants that affect its behavior:
//! 
//! - `BLOCK_COMPRESSABLE_RATIO`: 0.9 (threshold for good vs poor compression)
//! - `INITIAL_COMPRESSION_VALUE`: -80 (starting compression value)
//! - `COMPRESSIBLE_BLOCK_WEIGHT`: -10 (adjustment for good compression)
//! - `NON_COMPRESSIBLE_BLOCK_WEIGHT`: 4 (adjustment for poor compression)
//! - `SKIP_COMPRESSION_BLOCK_WEIGHT`: -1 (adjustment when skipping)
//! 
//! These parameters create a system that:
//! - Starts optimistic (negative value = always compress)
//! - Quickly adapts to poor compression (small positive weight vs large negative)
//! - Gradually returns to compression attempts through skip penalties
//! 
//! ## Performance Characteristics
//! 
//! - **Lock-free**: All operations use atomic compare-and-swap loops
//! - **Memory efficient**: Only three atomic values per instance
//! - **Low overhead**: Minimal computation per decision
//! - **Scalable**: Performance doesn't degrade with thread count

pub mod mvcompression;

pub use mvcompression::MVCompression;
