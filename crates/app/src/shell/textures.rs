use super::*;

impl Browser {
    pub(super) fn insert_texture(&mut self, key: String, handle: TextureHandle) {
        self.texture_order.retain(|have| have != &key);
        self.texture_order.push_back(key.clone());
        self.textures.insert(key, handle);
        // Lightbox images can be 16 MiB each; keep only the two most recent.
        while self
            .textures
            .keys()
            .filter(|key| key.starts_with("full:"))
            .count()
            > 2
        {
            let Some(index) = self
                .texture_order
                .iter()
                .position(|key| key.starts_with("full:"))
            else {
                break;
            };
            if let Some(old) = self.texture_order.remove(index) {
                self.textures.remove(&old);
            }
        }
        while self.textures.len() > TEXTURE_CAP {
            if let Some(old) = self.texture_order.pop_front() {
                self.textures.remove(&old);
            } else {
                break;
            }
        }
    }

    pub(super) fn texture(&mut self, key: &str) -> Option<TextureHandle> {
        let texture = self.textures.get(key)?.clone();
        self.texture_order.retain(|have| have != key);
        self.texture_order.push_back(key.into());
        Some(texture)
    }

    pub(super) fn decode_later(&mut self, key: String, path: PathBuf) {
        self.decode_later_scaled(key, path, 512);
    }

    pub(super) fn decode_memory_later(
        &mut self,
        key: String,
        name: String,
        bytes: Arc<[u8]>,
        max_edge: u32,
    ) {
        if self.decoding.len() >= 4
            || self.textures.contains_key(&key)
            || self.decode_failed.contains(&key)
            || !self.decoding.insert(key.clone())
        {
            return;
        }
        let tx = self.pixel_tx.clone();
        std::thread::spawn(move || {
            let pixels = match engine::thumbs::load_media_rgba_bytes(&bytes, &name, max_edge) {
                Ok((width, height, rgba)) => Pixels {
                    key,
                    width,
                    height,
                    rgba,
                },
                Err(_) => Pixels {
                    key,
                    width: 0,
                    height: 0,
                    rgba: Vec::new(),
                },
            };
            let _ = tx.send(pixels);
        });
    }

    pub(super) fn decode_later_scaled(&mut self, key: String, path: PathBuf, max_edge: u32) {
        if self.decoding.len() >= 4
            || self.textures.contains_key(&key)
            || self.decode_failed.contains(&key)
            || !self.decoding.insert(key.clone())
        {
            return;
        }
        let tx = self.pixel_tx.clone();
        std::thread::spawn(move || {
            let pixels = match engine::thumbs::load_rgba_scaled(&path, max_edge) {
                Ok((width, height, rgba)) => Pixels {
                    key,
                    width,
                    height,
                    rgba,
                },
                Err(_) => Pixels {
                    key,
                    width: 0,
                    height: 0,
                    rgba: Vec::new(),
                },
            };
            let _ = tx.send(pixels);
        });
    }
}
