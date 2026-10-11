use anyhow::{Context, Result};
use image::imageops::thumbnail;
use image::DynamicImage;
use prpr::ext::SafeTexture;
use std::future::Future;
use std::path::Path;

pub const THUMBNAIL_WIDTH: u32 = 347;
pub const THUMBNAIL_HEIGHT: u32 = 200;
// 低内存设备（Android getMemoryClass() 较小）用更小的封面，避免选歌页 OOM
const THUMBNAIL_WIDTH_LOW: u32 = 160;
const THUMBNAIL_HEIGHT_LOW: u32 = 92;

/// 按设备内存自动选择封面尺寸：普通设备保持清晰，低内存设备缩到 160x92。
pub fn thumbnail_dims() -> (u32, u32) {
    if crate::low_memory() {
        (THUMBNAIL_WIDTH_LOW, THUMBNAIL_HEIGHT_LOW)
    } else {
        (THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
    }
}

pub struct Images;
impl Images {
    pub fn into_texture(tex: (DynamicImage, Option<DynamicImage>)) -> (SafeTexture, SafeTexture) {
        match tex {
            (thumb, Some(full)) => (thumb.into(), full.into()),
            (thumb, None) => {
                let tex: SafeTexture = thumb.into();
                (tex.clone(), tex)
            }
        }
    }

    pub fn thumbnail(image: &DynamicImage) -> DynamicImage {
        let (_, th) = thumbnail_dims();
        let width = (image.width() as f32 / image.height() as f32 * th as f32).ceil() as u32;
        DynamicImage::ImageRgba8(thumbnail(image, width, th))
    }

    pub async fn local_or_else(path: impl AsRef<Path>, task: impl Future<Output = Result<DynamicImage>>) -> Result<DynamicImage> {
        let path = path.as_ref();
        Ok(if path.exists() {
            image::load_from_memory(&tokio::fs::read(path).await.context("Failed to read image")?)?
        } else {
            let image = task.await?;
            #[cfg(not(target_env = "ohos"))]
            {
                image.save_with_format(path, image::ImageFormat::Jpeg).context("Failed to save image")?;
            }
            #[cfg(target_env = "ohos")]
            {
                let rgb_image = DynamicImage::ImageRgb8(image.to_rgb8());
                rgb_image
                    .save_with_format(path, image::ImageFormat::Jpeg)
                    .context("Failed to save image")?;
            }
            image
        })
    }
}
