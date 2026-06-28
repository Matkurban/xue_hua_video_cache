#[flutter_rust_bridge::frb]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    Android,
    Ios,
    Other,
}

impl PlatformKind {
    pub fn is_android(&self) -> bool {
        matches!(self, PlatformKind::Android)
    }

    pub fn is_ios(&self) -> bool {
        matches!(self, PlatformKind::Ios)
    }
}
