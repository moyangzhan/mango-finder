use crate::document_loaders::open_doc;
use crate::global::ODP_EXTS;
use crate::traits::document_loader::DocumentLoader;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub struct OdpLoader {
    exts: Vec<String>,
}

impl Default for OdpLoader {
    fn default() -> Self {
        OdpLoader {
            exts: ODP_EXTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl DocumentLoader for OdpLoader {
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
        let text = open_doc::read_data(
            path,
            "content.xml",
            &["text:p", "text:span"],
            max_load_chars,
        )?;

        Ok(text)
    }
    fn load_file_max(&self, _file: &std::fs::File, _max_load_chars: usize) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "load_file_max with File is not supported for odp files, use load_max() with Path instead",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantiate() {
        let _ = OdpLoader::default().load_max(Path::new("samples/sample.odp"), 1000);
    }

    #[test]
    fn read() {
        let data = OdpLoader::default()
            .load_max(Path::new("samples/sample.odp"), 1000)
            .unwrap();
        println!("len: {}, data: {}", data.len(), data);
    }
}
