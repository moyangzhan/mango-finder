use crate::global::ODS_EXTS;
use crate::traits::document_loader::DocumentLoader;
use std::io;
use std::path::Path;

use super::open_doc;

#[derive(Debug)]
pub struct OdsLoader {
    exts: Vec<String>,
}

impl Default for OdsLoader {
    fn default() -> Self {
        Self {
            exts: ODS_EXTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl DocumentLoader for OdsLoader {
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
            "load_file_max is not supported for ODS files, use load_max with Path instead",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantiate() {
        let _ = OdsLoader::default().load_max(Path::new("samples/sample.ods"), 1000);
    }
}
