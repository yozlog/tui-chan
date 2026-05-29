use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use tui::text::Spans;
use tokio::runtime::Handle;
use image::GenericImageView;
use crate::event::Key;
use crate::event::Event;

/// Pre-rendered image spans and Base64 encoded payload for high-res protocols
#[derive(Clone)]
pub struct CachedImage {
    pub inline: Arc<Vec<Spans<'static>>>,
    pub split: Arc<Vec<Spans<'static>>>,
    pub base64_png: Arc<String>,
    pub width: u32,
    pub height: u32,
}

/// Status of the image inside the cache
#[derive(Clone)]
pub enum ImageStatus {
    Loaded(Arc<CachedImage>),
    Loading,
    Failed,
}

/// Thread-safe in-memory cache for rendered image half-blocks and Base64 PNGs
#[derive(Clone)]
pub struct ImageCache {
    /// In-memory cache map of URL to pre-rendered CachedImage
    cache: Arc<Mutex<HashMap<String, Arc<CachedImage>>>>,
    /// Keep track of URLs currently being downloaded to prevent redundant requests
    downloading: Arc<Mutex<HashSet<String>>>,
    /// Keep track of failed downloads to avoid infinite download/decode retry loops
    failed: Arc<Mutex<HashSet<String>>>,
    /// Tokio runtime handle to spawn background async tasks
    tokio_handle: Handle,
    /// Channel sender to trigger TUI loop refresh when downloads complete
    tx: mpsc::Sender<Event<Key>>,
    /// Shared HTTP client for connection pooling
    client: reqwest::Client,
}

impl ImageCache {
    /// Create a new `ImageCache` using a Tokio runtime handle and event sender
    pub fn new(tokio_handle: Handle, tx: mpsc::Sender<Event<Key>>, client: reqwest::Client) -> Self {
        ImageCache {
            cache: Arc::new(Mutex::new(HashMap::new())),
            downloading: Arc::new(Mutex::new(HashSet::new())),
            failed: Arc::new(Mutex::new(HashSet::new())),
            tokio_handle,
            tx,
            client,
        }
    }

    pub fn get_image(&self, url: &str, is_priority: bool) -> ImageStatus {
        // Skip unsupported video and media formats (like WebM and MP4)
        if url.ends_with(".webm") || url.ends_with(".mp4") {
            return ImageStatus::Failed;
        }

        // First check if it is already in the cache
        {
            let cache = self.cache.lock().unwrap();
            if let Some(cached_img) = cache.get(url) {
                return ImageStatus::Loaded(cached_img.clone());
            }
        }

        // Check if this URL previously failed to download/decode
        {
            let failed = self.failed.lock().unwrap();
            if failed.contains(url) {
                return ImageStatus::Failed;
            }
        }

        // Check if it's currently downloading
        let mut downloading = self.downloading.lock().unwrap();
        if downloading.contains(url) {
            return ImageStatus::Loading;
        }

        // Throttling: limit maximum concurrent background downloads to 4,
        // but PRIORITIZED active selections bypass this limit to load instantly!
        if !is_priority && downloading.len() >= 4 {
            return ImageStatus::Loading;
        }

        downloading.insert(url.to_string());

        let url_clone = url.to_string();
        let cache_clone = self.cache.clone();
        let downloading_clone = self.downloading.clone();
        let failed_clone = self.failed.clone();
        let tx_clone = self.tx.clone();
        let client_clone = self.client.clone();

        // Spawn background asynchronous download and rendering task
        self.tokio_handle.spawn(async move {
            let result = async {
                // Fetch image bytes
                let response = client_clone.get(&url_clone).send().await?;
                let bytes = response.bytes().await?;
                
                // Decode image
                let img = image::load_from_memory(&bytes)?;
                
                // Limit dimensions to maximum 600px to avoid terminal buffer rendering lag
                let (orig_w, orig_h) = img.dimensions();
                let resized = if orig_w > 600 || orig_h > 600 {
                    img.thumbnail(600, 600)
                } else {
                    img
                };
                let (w, h) = resized.dimensions();

                // Encode to PNG bytes
                let mut png_bytes = Vec::new();
                resized.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageOutputFormat::Png)?;

                // Base64 encode PNG
                let base64_png = crate::graphics::base64_encode(&png_bytes);

                // Pre-render inline size (14x7 cells) in the background thread
                let inline_blocks = crate::image_renderer::render_half_blocks(&resized, 14, 7);
                
                // Pre-render split size (30x15 cells) in the background thread
                let split_blocks = crate::image_renderer::render_half_blocks(&resized, 30, 15);
                
                Ok::<CachedImage, Box<dyn std::error::Error + Send + Sync>>(CachedImage {
                    inline: Arc::new(inline_blocks),
                    split: Arc::new(split_blocks),
                    base64_png: Arc::new(base64_png),
                    width: w,
                    height: h,
                })
            }
            .await;

            match result {
                Ok(cached_img) => {
                    let mut cache = cache_clone.lock().unwrap();
                    cache.insert(url_clone.clone(), Arc::new(cached_img));
                }
                Err(_err) => {
                    // Mark as failed so we don't try to download/decode this URL again in an infinite loop
                    let mut failed = failed_clone.lock().unwrap();
                    failed.insert(url_clone.clone());
                }
            }

            // Remove from downloading set regardless of success/failure
            let mut downloading = downloading_clone.lock().unwrap();
            downloading.remove(&url_clone);

            // Active wakeup: notify main TUI thread to redraw and show loaded image
            let _ = tx_clone.send(Event::Tick);
        });

        ImageStatus::Loading
    }
}
