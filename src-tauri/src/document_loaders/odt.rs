use super::open_doc;
use crate::global::ODT_EXTS;
use crate::traits::document_loader::DocumentLoader;
use std::io;
use std::path::Path;

/// Open Document Text
#[derive(Debug)]
pub struct OdtLoader {
    exts: Vec<String>,
}

impl Default for OdtLoader {
    fn default() -> Self {
        Self {
            exts: ODT_EXTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl DocumentLoader for OdtLoader {
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
        let text = open_doc::read_data(path, "content.xml", &["text:p"], max_load_chars)?;
        Ok(text)
    }
    fn load_file_max(&self, _file: &std::fs::File, _max_load_chars: usize) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "load_file_max with File is not supported for odt files, use load_max() with Path instead",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantiate() {
        let _ = OdtLoader::default().load_max(Path::new("samples/sample.odt"), 1000);
    }

    #[test]
    fn read() {
        let data = OdtLoader::default()
            .load_max(Path::new("samples/sample.odt"), 1000)
            .unwrap();
        println!("len: {}, data: {}", data.len(), data);
    }
}
