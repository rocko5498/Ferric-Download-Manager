// Manual check: `cargo run -p ferric-extract --example yt_formats -- <url>` prints the format count.
#[tokio::main]
async fn main() {
    let url = std::env::args().nth(1).expect("url");
    let r = ferric_extract::ytdlp::extract(&url, Default::default())
        .await
        .expect("extract");
    println!("formats: {}", r.formats.len());
}
