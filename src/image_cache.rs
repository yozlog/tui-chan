use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tui::text::Spans;
use tokio::runtime::Handle;

/// Pre-rendered image spans for different layout modes
#[derive(Clone)]
pub struct CachedImage {
    pub inline: Arc<Vec<Spans<'static>>>,
    pub split: Arc<Vec<Spans<'static>>>,
}

/// Status of the image inside the cache
#[derive(Clone)]
pub enum ImageStatus {
    Loaded(Arc<CachedImage>),
    Loading,
    Failed,
}

/// Thread-safe in-memory cache for rendered image half-blocks
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
}

impl ImageCache {
    /// Create a new `ImageCache` using a Tokio runtime handle
    pub fn new(tokio_handle: Handle) -> Self {
        ImageCache {
            cache: Arc::new(Mutex::new(HashMap::new())),
            downloading: Arc::new(Mutex::new(HashSet::new())),
            failed: Arc::new(Mutex::new(HashSet::new())),
            tokio_handle,
        }
    }

    /// Retrieve pre-rendered image status.
    /// If not in cache, not downloading, and not previously failed, spawns an async background task to download, decode, and render both sizes.
    pub fn get_image(&self, url: &str) -> ImageStatus {
        // Skip unsupported video formats like WebM
        if url.ends_with(".webm") {
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

        // Throttling: limit maximum concurrent background downloads to 4
        if downloading.len() >= 4 {
            return ImageStatus::Loading;
        }

        downloading.insert(url.to_string());

        let url_clone = url.to_string();
        let cache_clone = self.cache.clone();
        let downloading_clone = self.downloading.clone();
        let failed_clone = self.failed.clone();

        // Spawn background asynchronous download and rendering task
        self.tokio_handle.spawn(async move {
            let result = async {
                // Fetch image bytes
                let response = reqwest::get(&url_clone).await?;
                let bytes = response.bytes().await?;
                
                // Decode image
                let img = image::load_from_memory(&bytes)?;
                
                // Pre-render inline size (14x7 cells) in the background thread
                let inline_blocks = crate::image_renderer::render_half_blocks(&img, 14, 7);
                
                // Pre-render split size (30x15 cells) in the background thread
                let split_blocks = crate::image_renderer::render_half_blocks(&img, 30, 15);
                
                Ok::<CachedImage, Box<dyn std::error::Error + Send + Sync>>(CachedImage {
                    inline: Arc::new(inline_blocks),
                    split: Arc::new(split_blocks),
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
        });

        ImageStatus::Loading
    }
}
