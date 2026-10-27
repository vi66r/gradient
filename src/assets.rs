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
        ArrowUpRight,
        Bot,
        LoaderCircle,
        LockKeyhole,
        CloudOff,
        Search,
        Info,
        Play,
        Pause,
        Volume2,
        VolumeX,
        SkipBack,
        SkipForward,
        Download,
        AppWindow,
        Puzzle,
        Clock,
        Eraser,
        Trash,
        RotateCcw
    ]
);

/// Pop a tab out into its own window: two square corners turned 45°,
/// pointing apart (<>).
pub const POP_OUT: &str = "icons/browser-pop-out.svg";
/// Return a page to the browser window: the corners pointing together (><).
pub const POP_IN: &str = "icons/browser-pop-in.svg";

const POP_OUT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6"/><path d="M10 20H4v-6"/></svg>"#;
const POP_IN_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4v6h6"/><path d="M10 20v-6H4"/></svg>"#;

pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            POP_OUT => return Ok(Some(Cow::Borrowed(POP_OUT_SVG.as_bytes()))),
            POP_IN => return Ok(Some(Cow::Borrowed(POP_IN_SVG.as_bytes()))),
            _ => {}
        }
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
