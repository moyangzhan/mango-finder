use super::{image_filename, images_dir_for};
use crate::global::{
    ANYTOMD_EXTRA_EXTS, DOCX_EXTS, EXCEL_EXTS, MAX_DOCUMENT_LOAD_CHARS, PLAIN_TEXT_EXTS, PPTX_EXTS,
};
use crate::local_image_parser;
use crate::ocr_service;
use crate::traits::document_loader::DocumentLoader;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub struct AnyToMdLoader {
    exts: Vec<String>,
}

impl Default for AnyToMdLoader {
    fn default() -> Self {
        let exts: Vec<&[&str]> = vec![
            DOCX_EXTS,
            PPTX_EXTS,
            EXCEL_EXTS,
            PLAIN_TEXT_EXTS,
            ANYTOMD_EXTRA_EXTS,
        ];
        Self {
            exts: exts
                .into_iter()
                .flat_map(|e| e.iter().map(|s| s.to_string()))
                .collect(),
        }
    }
}

impl DocumentLoader for AnyToMdLoader {
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
        self.load_doc(path, &images_dir, 0, max_load_chars)
    }

    fn load_max_with_id(
        &self,
        path: &Path,
        file_id: i64,
        _file_name: &str,
        max_load_chars: usize,
    ) -> io::Result<String> {
        let (images_dir, _guard) = images_dir_for(file_id);
        self.load_doc(path, &images_dir, file_id, max_load_chars)
    }

    fn load_file_max(&self, _file: &std::fs::File, _max_load_chars: usize) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "load_file_max is not supported for AnyToMdLoader, use load_max with Path instead",
        ))
    }
}

impl AnyToMdLoader {
    fn load_doc(
        &self,
        path: &Path,
        images_dir: &Path,
        file_id: i64,
        max_load_chars: usize,
    ) -> io::Result<String> {
        // Skip files over 100 MB — too large for in-memory conversion
        const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
        if let Ok(meta) = fs::metadata(path) {
            if meta.len() > MAX_FILE_SIZE {
                log::warn!(
                    "Skipping file too large for parsing ({} bytes): {}",
                    meta.len(),
                    path.display()
                );
                return Ok(String::new());
            }
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Short-circuit for plain text files: read directly without anytomd parsing
        if PLAIN_TEXT_EXTS.contains(&ext.as_str()) {
            return load_plain_text(path, max_load_chars);
        }

        let options = anytomd::ConversionOptions {
            extract_images: true,
            max_total_image_bytes: 10 * 1024 * 1024,
            ..Default::default()
        };

        let result = anytomd::convert_file(path, &options).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse document {}: {}", path.display(), e),
            )
        })?;

        let mut content = result.markdown;

        if !result.images.is_empty() {
            // Dir folder name doubles as the markdown rel-path segment (bucket
            // for indexed parses, per-query temp dir for external ones)
            // 目录名兼作 markdown 相对路径段(索引解析为桶名,外部解析为查询级临时目录)
            let bucket = images_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("0000")
                .to_string();
            let img_rel_prefix = format!("../../extracted_images/{}/", bucket);

            if !images_dir.exists() {
                fs::create_dir_all(images_dir).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("Failed to create images dir: {}", e),
                    )
                })?;
            }

            // Fix image references: replace original filenames with id-prefixed names
            for (filename, _) in &result.images {
                let img_name = if file_id > 0 {
                    image_filename(file_id, filename)
                } else {
                    filename.to_string()
                };
                let old_ref = format!("]({})", filename);
                let new_ref = format!("]({}{})", img_rel_prefix, img_name);
                content = content.replace(&old_ref, &new_ref);
            }

            let mut image_texts = Vec::new();
            for (filename, bytes) in &result.images {
                let img_name = if file_id > 0 {
                    image_filename(file_id, filename)
                } else {
                    filename.to_string()
                };
                let img_path = images_dir.join(&img_name);
                fs::write(&img_path, bytes)?;

                let mut parts = Vec::new();
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

            if !image_texts.is_empty() {
                content.push_str("\n\n");
                content.push_str(&image_texts.join("\n\n"));
            }
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

fn load_plain_text(path: &Path, max_load_chars: usize) -> io::Result<String> {
    use std::io::Read;

    let limit = if max_load_chars > 0 {
        max_load_chars
    } else {
        MAX_DOCUMENT_LOAD_CHARS
    };

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut content = String::new();

    reader
        .take((limit as u64) * 4)
        .read_to_string(&mut content)?;
    super::truncate_to_char_limit(&mut content, limit);

    Ok(content)
}
