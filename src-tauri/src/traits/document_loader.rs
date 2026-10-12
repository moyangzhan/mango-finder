use std::io;
use std::path::Path;

pub trait DocumentLoader {
    fn get_exts(&self) -> &[String];
    fn add_ext(&mut self, ext: String);
    fn load(&self, path: &Path) -> io::Result<String>;

    /// Loads content from a file up to a maximum number of chars(not characters).
    fn load_max(&self, path: &Path, max_load_chars: usize) -> io::Result<String>;

    /// Load with file ID for human-readable image directory names.
    /// Defaults to `load_max` for loaders that don't need the ID.
    fn load_max_with_id(
        &self,
        path: &Path,
        file_id: i64,
        file_name: &str,
        max_load_chars: usize,
    ) -> io::Result<String> {
        let _ = (file_id, file_name);
        self.load_max(path, max_load_chars)
    }

    fn load_file_max(&self, file: &std::fs::File, max_load_chars: usize) -> io::Result<String>;
}

pub trait OpenOfficeDoc {
    fn load<P: AsRef<Path>>(path: P, max_load_chars: usize) -> io::Result<String>;
    fn load_max<P: AsRef<Path>>(path: P, max_load_chars: usize) -> io::Result<String>;
}
