use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

gpui::assets::icon_assets!(
    ExtraIcons,
    [
        Terminal,
        Check,
        Code,
        Compass,
        Box,
        X,
        Lock,
        Moon,
        ArrowLeft,
        ArrowUpRight
    ]
);

pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        Ok(paths)
    }
}
