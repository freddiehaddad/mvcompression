use mvcompression::MVCompression;

/// This example demonstrates the basic usage of the MVCompression algorithm.
/// It simulates processing 30 blocks of data where compression is ineffective
/// (compressed size equals uncompressed size), showing how the algorithm
/// learns to skip compression over time.
fn main() {
    println!("MVCompression Algorithm Demo");
    println!("============================");
    println!("Simulating compression of 30 blocks (1000 bytes each)");
    println!("Note: All blocks have compression ratio = 1.0 (no compression achieved)\n");

    let mvc = MVCompression::new();
    let mut skip_count = 0;
    let mut compress_count = 0;

    for i in 1..=30 {
        let block = i;
        let uncompressed = 1000;
        let compressed = 1000; // Simulating no compression achieved
        
        if mvc.should_skip_compression(uncompressed) {
            println!("Block {}: SKIPPED compression (size: {} bytes)", block, uncompressed);
            skip_count += 1;
        } else {
            mvc.update_compression_ratio(compressed, uncompressed);
            println!("Block {}: COMPRESSED {} -> {} bytes (ratio: {:.2})", 
                    block, uncompressed, compressed, 
                    compressed as f32 / uncompressed as f32);
            compress_count += 1;
        }
        
        // Show algorithm state every 5 blocks
        if i % 5 == 0 {
            println!("  → Compression value: {}, Avg uncompressed: {}", 
                    mvc.get_compression_value(), mvc.get_uncompressed_average());
        }
    }

    println!("\n=== Final Results ===");
    println!("Blocks compressed: {}", compress_count);
    println!("Blocks skipped: {}", skip_count);
    println!("Final compression value: {}", mvc.get_compression_value());
    println!("Final uncompressed average: {}", mvc.get_uncompressed_average());
    
    if skip_count > 0 {
        println!("\n✓ Algorithm successfully learned to skip ineffective compression!");
    } else {
        println!("\n! Algorithm did not skip any compressions in this run.");
    }
}
