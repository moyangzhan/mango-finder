use super::{image_filename, images_dir_for};
use crate::global::{MAX_DOCUMENT_LOAD_CHARS, PDF_EXTS};
use crate::local_image_parser;
use crate::ocr_service;
use crate::traits::document_loader::DocumentLoader;
use lopdf::Document as LopdfDocument;
use lopdf::Object;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct PdfPlumberLoader {
    exts: Vec<String>,
}

impl Default for PdfPlumberLoader {
    fn default() -> Self {
        Self {
            exts: PDF_EXTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl DocumentLoader for PdfPlumberLoader {
    fn get_exts(&self) -> &[String] {
        &self.exts
    }

    fn add_ext(&mut self, ext: String) {
        self.exts.push(ext);
    }

    fn load(&self, path: &Path) -> io::Result<String> {
        self.load_max(path, 0)
    }

    fn load_max(&self, path: &Path, max_load_chars: usize) -> io::Result<String> {
        let (images_dir, _guard) = images_dir_for(0);
        self.load_pdf(path, &images_dir, 0, max_load_chars)
    }

    fn load_max_with_id(
        &self,
        path: &Path,
        file_id: i64,
        _file_name: &str,
        max_load_chars: usize,
    ) -> io::Result<String> {
        let (images_dir, _guard) = images_dir_for(file_id);
        self.load_pdf(path, &images_dir, file_id, max_load_chars)
    }

    fn load_file_max(&self, _file: &std::fs::File, _max_load_chars: usize) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "load_file_max is not supported for PdfPlumberLoader, use load_max with Path instead",
        ))
    }
}

impl PdfPlumberLoader {
    fn load_pdf(
        &self,
        path: &Path,
        images_dir: &Path,
        file_id: i64,
        max_load_chars: usize,
    ) -> io::Result<String> {
        let pdf = pdfplumber::Pdf::open_file(path, None).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to open PDF {}: {}", path.display(), e),
            )
        })?;

        let mut pages_content = Vec::new();
        let mut image_texts = Vec::new();
        let mut failed_pages = Vec::new();

        // Dir folder name doubles as the markdown rel-path segment (bucket for indexed parses, per-query temp dir for external ones)
        let bucket = images_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("0000")
            .to_string();
        let img_rel_prefix = format!("../../extracted_images/{}/", bucket);

        for page_idx in 0..pdf.page_count() {
            let page = match pdf.page(page_idx) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!(
                        "Failed to read page {} of {}: {}",
                        page_idx,
                        path.display(),
                        e
                    );
                    failed_pages.push(page_idx);
                    continue;
                }
            };

            let chars = page.chars();
            let tables = page.find_tables(&pdfplumber::TableSettings::default());
            let page_text = pdfplumber::MarkdownRenderer::render(
                &chars,
                &tables,
                &pdfplumber::MarkdownOptions::default(),
            );

            if !page_text.is_empty() {
                pages_content.push(page_text);
            }

            // Extract images and run BLIP + OCR
            let images = page.images();
            if !images.is_empty() {
                if let Err(e) = fs::create_dir_all(images_dir) {
                    log::warn!("Failed to create images dir: {}", e);
                }

                for (img_idx, img) in images.iter().enumerate() {
                    if let Ok(img_content) = pdf.extract_image_content(page_idx, &img.name) {
                        let ext = match img_content.format {
                            pdfplumber::ImageFormat::Jpeg => "jpg",
                            pdfplumber::ImageFormat::Png => "png",
                            _ => "bin",
                        };
                        let suffix = format!("p{}_img{}.{}", page_idx, img_idx, ext);
                        let img_name = if file_id > 0 {
                            image_filename(file_id, &suffix)
                        } else {
                            suffix
                        };
                        let img_path = images_dir.join(&img_name);
                        if fs::write(&img_path, &img_content.data).is_ok() {
                            let img_ref =
                                format!("![{}]({}{})", img.name, img_rel_prefix, img_name);
                            let mut parts = vec![img_ref];

                            let blip_caption = local_image_parser::generate_caption(&img_path);
                            let ocr_text = ocr_service::recognize_file(&img_path);
                            if !blip_caption.is_empty() {
                                parts.push(format!("**Image Description:** {}", blip_caption));
                            }
                            if !ocr_text.is_empty() {
                                parts.push(format!("**OCR Text:** {}", ocr_text));
                            }
                            image_texts.push(parts.join("\n\n"));
                        }
                    }
                }
            }
        }

        // Fallback: extract images from failed pages using lopdf directly
        if !failed_pages.is_empty() {
            if let Ok(lopdf_doc) = LopdfDocument::load(path) {
                let _ = fs::create_dir_all(images_dir);
                for page_idx in &failed_pages {
                    extract_images_from_page(
                        &lopdf_doc,
                        *page_idx,
                        images_dir,
                        file_id,
                        &img_rel_prefix,
                        &mut image_texts,
                    );
                }
            }
        }

        let mut content = pages_content.join("\n\n---\n\n");

        if !image_texts.is_empty() {
            content.push_str("\n\n");
            content.push_str(&image_texts.join("\n\n"));
        }

        let limit = if max_load_chars > 0 {
            max_load_chars
        } else {
            MAX_DOCUMENT_LOAD_CHARS
        };

        super::truncate_to_char_limit(&mut content, limit);

        Ok(content)
    }
}

/// Fallback image extraction using lopdf directly.
fn extract_images_from_page(
    doc: &LopdfDocument,
    page_idx: usize,
    images_dir: &Path,
    file_id: i64,
    img_rel_prefix: &str,
    image_texts: &mut Vec<String>,
) {
    let page_id = match doc.get_pages().get(&(page_idx as u32 + 1)) {
        Some(id) => *id,
        None => {
            log::warn!("Fallback: page {} not found in lopdf pages", page_idx);
            return;
        }
    };

    let (resources_dict, resource_ids) = match doc.get_page_resources(page_id) {
        Ok(r) => r,
        Err(e) => {
            log::warn!(
                "Fallback: get_page_resources failed for page {}: {}",
                page_idx,
                e
            );
            return;
        }
    };

    // Try to find XObject from the page's own resources dict, or fall back to traversing resource_ids (inherited from parent nodes)
    let xobjects = resources_dict
        .and_then(|dict| find_xobject_dict(doc, dict).ok())
        .or_else(|| {
            for &rid in &resource_ids {
                if let Ok(dict) = doc.get_dictionary(rid) {
                    if let Ok(xo) = find_xobject_dict(doc, &dict) {
                        return Some(xo);
                    }
                }
            }
            None
        });

    let xobjects = match xobjects {
        Some(xo) => xo,
        None => {
            log::debug!("Fallback: no XObject found for page {}", page_idx);
            return;
        }
    };

    log::debug!(
        "Fallback: page {} has {} XObjects",
        page_idx,
        xobjects.len()
    );

    let mut img_count = 0usize;
    for (name, obj_ref) in xobjects.iter() {
        let obj_id = match obj_ref.as_reference() {
            Ok(id) => id,
            Err(_) => continue,
        };

        let obj = match doc.get_object(obj_id) {
            Ok(o) => o,
            Err(_) => continue,
        };

        let stream = match obj.as_stream() {
            Ok(s) => s,
            Err(_) => continue,
        };

        let subtype = stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|s| s.as_name_str().ok());
        if subtype.as_deref() != Some("Image") {
            continue;
        }

        let filters = stream.filters().unwrap_or_default();
        let ext = match filters.first().map(|s| s.as_str()) {
            Some("DCTDecode") => "jpg",
            Some("FlateDecode") => "png",
            Some("CCITTFaxDecode") => "tif",
            Some("JPXDecode") => "jp2",
            _ => "bin",
        };

        let (data, decompressed) = match stream.decompressed_content() {
            Ok(d) => (d, true),
            Err(e) => {
                log::debug!(
                    "Fallback: decompression failed for '{}' on page {}: {}, trying manual zlib decode ({} bytes)",
                    String::from_utf8_lossy(name),
                    page_idx,
                    e,
                    stream.content.len()
                );
                // lopdf's decompressed_content() can fail on some PDFs.
                // Manually decompress using flate2 as a fallback.
                match decompress_zlib(&stream.content) {
                    Some(d) => (d, true),
                    None => {
                        log::debug!(
                            "Fallback: manual zlib decode also failed for '{}' on page {}, using raw content",
                            String::from_utf8_lossy(name),
                            page_idx
                        );
                        (stream.content.clone(), false)
                    }
                }
            }
        };

        if data.is_empty() {
            continue;
        }

        log::debug!(
            "Fallback: image '{}' page {} — {} bytes, decompressed: {}, filters: {:?}, first bytes: {:02X?}",
            String::from_utf8_lossy(name),
            page_idx,
            data.len(),
            decompressed,
            stream.filters().unwrap_or_default(),
            &data[..data.len().min(8)]
        );

        // Check if decompressed data is actually JPEG (common in double-filtered PDFs:
        // FlateDecode wrapping DCTDecode). If so, save as .jpg directly.
        let (final_ext, final_data) = if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
            ("jpg", &data)
        } else {
            (ext, &data)
        };

        let suffix = format!("p{}_fallback_img{}.{}", page_idx, img_count, final_ext);
        let img_name = if file_id > 0 {
            image_filename(file_id, &suffix)
        } else {
            suffix
        };
        let img_path = images_dir.join(&img_name);

        let final_path = if final_ext == "jpg" {
            // JPEG data — write directly
            if fs::write(&img_path, final_data).is_err() {
                continue;
            }
            img_path.clone()
        } else {
            // Raw pixel data — convert to PNG
            match convert_stream_to_png(final_data, &stream.dict, images_dir, &img_name) {
                Some(png_path) => png_path,
                None => {
                    log::warn!(
                        "Fallback: failed to convert image '{}' on page {} ({} bytes, colorspace: {:?})",
                        String::from_utf8_lossy(name),
                        page_idx,
                        final_data.len(),
                        stream
                            .dict
                            .get(b"ColorSpace")
                            .ok()
                            .and_then(|v| v.as_name_str().ok())
                    );
                    continue;
                }
            }
        };

        let name_str = String::from_utf8_lossy(name);
        log::info!(
            "Extracted fallback image '{}' ({} bytes) from page {}",
            name_str,
            data.len(),
            page_idx
        );

        let final_filename = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&img_name);
        let img_ref = format!("![{}]({}{})", name_str, img_rel_prefix, final_filename);

        let mut parts = vec![img_ref];
        let blip_caption = local_image_parser::generate_caption(&final_path);
        let ocr_text = ocr_service::recognize_file(&final_path);
        if !blip_caption.is_empty() {
            parts.push(format!("**Image Description:** {}", blip_caption));
        }
        if !ocr_text.is_empty() {
            parts.push(format!("**OCR Text:** {}", ocr_text));
        }
        image_texts.push(parts.join("\n\n"));
        img_count += 1;
    }
}

fn convert_stream_to_png(
    data: &[u8],
    dict: &lopdf::Dictionary,
    images_dir: &Path,
    img_name: &str,
) -> Option<PathBuf> {
    let width: u32 = dict
        .get(b"Width")
        .ok()
        .and_then(|v| v.as_i64().ok())
        .and_then(|v| u32::try_from(v).ok())?;

    let height: u32 = dict
        .get(b"Height")
        .ok()
        .and_then(|v| v.as_i64().ok())
        .and_then(|v| u32::try_from(v).ok())?;

    let color_components: u32 = dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|cs| {
            match cs {
                Object::Name(name) => match name.as_slice() {
                    b"DeviceRGB" => Some(3u32),
                    b"DeviceGray" => Some(1u32),
                    b"DeviceCMYK" => Some(4u32),
                    _ => {
                        log::debug!(
                            "convert_stream_to_png: unsupported ColorSpace name: {}",
                            String::from_utf8_lossy(name)
                        );
                        None
                    }
                },
                Object::Array(arr) => {
                    // Try to get the first element as the color space family name
                    let family = arr.first().and_then(|o| o.as_name_str().ok());
                    log::debug!(
                        "convert_stream_to_png: array ColorSpace, family: {:?}",
                        family
                    );
                    match family.as_deref() {
                        Some("DeviceRGB") | Some("CalRGB") | Some("ICCBased") => Some(3u32),
                        Some("DeviceGray") | Some("CalGray") => Some(1u32),
                        Some("DeviceCMYK") => Some(4u32),
                        _ => None,
                    }
                }
                _ => {
                    log::debug!(
                        "convert_stream_to_png: unexpected ColorSpace type: {:?}",
                        cs
                    );
                    None
                }
            }
        })
        .unwrap_or(3);

    let bits_per_component: u32 = dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|v| v.as_i64().ok())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(8);

    if bits_per_component != 8 {
        log::debug!(
            "convert_stream_to_png: unsupported bits_per_component: {}",
            bits_per_component
        );
        return None;
    }

    let expected_len = width as usize * height as usize * color_components as usize;
    if data.len() < expected_len {
        log::debug!(
            "convert_stream_to_png: data too short: {} < {} ({}x{}x{})",
            data.len(),
            expected_len,
            width,
            height,
            color_components
        );
        return None;
    }

    let png_name = img_name.replace(".bin", ".png").replace(".tif", ".png");
    let png_path = images_dir.join(&png_name);

    let img_result = match color_components {
        1 => {
            let img = image::GrayImage::from_raw(width, height, data[..expected_len].to_vec())?;
            image::DynamicImage::ImageLuma8(img)
        }
        3 => {
            let img = image::RgbImage::from_raw(width, height, data[..expected_len].to_vec())?;
            image::DynamicImage::ImageRgb8(img)
        }
        4 => {
            let rgb_data = cmyk_to_rgb(&data[..expected_len]);
            let img = image::RgbImage::from_raw(width, height, rgb_data)?;
            image::DynamicImage::ImageRgb8(img)
        }
        _ => return None,
    };

    img_result.save(&png_path).ok()?;
    Some(png_path)
}

fn cmyk_to_rgb(data: &[u8]) -> Vec<u8> {
    data.chunks_exact(4)
        .flat_map(|pixel| {
            let c = pixel[0] as f32 / 255.0;
            let m = pixel[1] as f32 / 255.0;
            let y = pixel[2] as f32 / 255.0;
            let k = pixel[3] as f32 / 255.0;
            let r = ((1.0 - c) * (1.0 - k) * 255.0) as u8;
            let g = ((1.0 - m) * (1.0 - k) * 255.0) as u8;
            let b = ((1.0 - y) * (1.0 - k) * 255.0) as u8;
            [r, g, b]
        })
        .collect()
}

/// Find the XObject dictionary from a Resources dictionary.
/// Handles both indirect references and inline dictionaries.
fn find_xobject_dict<'a>(
    doc: &'a LopdfDocument,
    resources: &'a lopdf::Dictionary,
) -> Result<&'a lopdf::Dictionary, ()> {
    let xobj_val = resources.get(b"XObject").map_err(|_| ())?;
    // Case 1: indirect reference
    if let Ok(ref_id) = xobj_val.as_reference() {
        return doc.get_dictionary(ref_id).map_err(|_| ());
    }
    // Case 2: direct dictionary
    xobj_val.as_dict().map_err(|_| ())
}

/// Manually decompress zlib/deflate data when lopdf's built-in decompression fails.
fn decompress_zlib(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    // zlib header: 78 01 (low), 78 5E (default), 78 9C (default), 78 DA (best) raw deflate (no header): try anyway
    let result = if data.len() >= 2 && data[0] == 0x78 {
        let mut decoder = flate2::read::ZlibDecoder::new(data);
        let mut buf = Vec::with_capacity(data.len() * 4);
        decoder.read_to_end(&mut buf).ok().map(|_| buf)
    } else {
        // Try raw deflate
        let mut decoder = flate2::read::DeflateDecoder::new(data);
        let mut buf = Vec::with_capacity(data.len() * 4);
        decoder.read_to_end(&mut buf).ok().map(|_| buf)
    };
    result.filter(|d| !d.is_empty())
}
