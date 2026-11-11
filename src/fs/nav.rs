use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Navigation {
    history: Vec<PathBuf>,
    index: usize,
}

impl Navigation {
    pub fn new(path: PathBuf) -> Self {
        let sanitized = sanitize_path(&path.to_string_lossy(), None);
        let initial = if sanitized.is_dir() {
            sanitized
        } else {
            PathBuf::from("C:\\")
        };
        Self {
            history: vec![initial],
            index: 0,
        }
    }

    pub fn current(&self) -> &PathBuf {
        &self.history[self.index]
    }

    #[allow(dead_code)]
    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn back(&mut self) -> Option<&PathBuf> {
        if self.can_back() {
            self.index -= 1;
            Some(self.current())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn can_forward(&self) -> bool {
        self.index + 1 < self.history.len()
    }

    pub fn forward(&mut self) -> Option<&PathBuf> {
        if self.can_forward() {
            self.index += 1;
            Some(self.current())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn can_parent(&self) -> bool {
        self.current().parent().is_some()
    }

    pub fn parent(&mut self) -> Option<&PathBuf> {
        let parent = self.current().parent()?.to_path_buf();
        if self.go_internal(parent) {
            Some(self.current())
        } else {
            None
        }
    }

    pub fn go_str(&mut self, input: &str) -> bool {
        let target = sanitize_path(input, Some(self.current()));
        self.go(target)
    }

    pub fn go(&mut self, path: PathBuf) -> bool {
        self.go_internal(path)
    }

    fn go_internal(&mut self, path: PathBuf) -> bool {
        let normalized = normalize_path(&path);
        if !normalized.is_dir() {
            return false;
        }
        if self.current() == &normalized {
            return true;
        }
        self.history.truncate(self.index + 1);
        self.history.push(normalized);
        self.index += 1;
        true
    }
}

pub fn sanitize_path(input: &str, base: Option<&Path>) -> PathBuf {
    let mut trimmed = input.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        if trimmed.len() >= 2 {
            trimmed = trimmed[1..trimmed.len() - 1].trim();
        }
    }

    if trimmed.is_empty() {
        return base
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("C:\\"));
    }

    let normalized_slash = trimmed.replace('/', "\\");
    let mut path = PathBuf::from(&normalized_slash);

    if path.is_relative() {
        if let Some(base_path) = base {
            path = base_path.join(path);
        }
    }

    let s = path.to_string_lossy();
    if s.len() == 2 && s.ends_with(':') {
        path = PathBuf::from(format!("{s}\\"));
    }

    normalize_path(&path)
}

pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for comp in path.components() {
        match comp {
            Component::Prefix(p) => {
                components.push(Component::Prefix(p));
            }
            Component::RootDir => {
                components.push(Component::RootDir);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = components.last() {
                    match last {
                        Component::Prefix(_) | Component::RootDir => {}
                        Component::Normal(_) => {
                            components.pop();
                        }
                        _ => {}
                    }
                }
            }
            Component::Normal(p) => {
                components.push(Component::Normal(p));
            }
        }
    }

    let mut result = PathBuf::new();
    for comp in components {
        result.push(comp.as_os_str());
    }

    let s = result.to_string_lossy();
    if s.len() == 2 && s.ends_with(':') {
        result.push("\\");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_and_normalize() {
        let p = sanitize_path(r#""C:\Windows\System32\..""#, None);
        assert_eq!(p, PathBuf::from(r#"C:\Windows"#));

        let p_rel = sanitize_path("..", Some(Path::new(r#"C:\Windows\System32"#)));
        assert_eq!(p_rel, PathBuf::from(r#"C:\Windows"#));

        let p_drive = sanitize_path("C:", None);
        assert_eq!(p_drive, PathBuf::from(r#"C:\"#));
    }

    #[test]
    fn test_navigation_history() {
        let root = PathBuf::from(r#"C:\"#);
        let mut nav = Navigation::new(root.clone());
        assert_eq!(nav.current(), &root);
        assert!(!nav.can_back());
        assert!(!nav.can_forward());

        let win = PathBuf::from(r#"C:\Windows"#);
        if win.is_dir() {
            assert!(nav.go(win.clone()));
            assert_eq!(nav.current(), &win);
            assert!(nav.can_back());

            assert_eq!(nav.back(), Some(&root));
            assert_eq!(nav.current(), &root);
            assert!(nav.can_forward());

            assert_eq!(nav.forward(), Some(&win));
            assert_eq!(nav.current(), &win);
        }
    }
}
