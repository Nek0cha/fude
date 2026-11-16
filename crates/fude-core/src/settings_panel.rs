//! 設定パネルでの変更（9b の設計書 `2026-11-15-settings-panel-hook-design.md` §5.2、§5.3）。
//! 変更を Settings に反映し、設定ファイルのどの行をどう書くかを決める。

use crate::settings::{CursorShape, Settings, ThemeId, line_height_for};
use crate::settings_edit::{EditError, EditValue, remove_value, set_value};

/// 不透明度のスライダーの範囲（設計書 §5.2）。
pub const OPACITY_MIN: f32 = 0.3;
pub const OPACITY_MAX: f32 = 1.0;
/// 文字の大きさの範囲（設定ファイルの検査と同じ）。
pub const FONT_SIZE_MIN: u32 = 8;
pub const FONT_SIZE_MAX: u32 = 32;
/// 行の高さの上限（設定ファイルの検査と同じ）。
pub const LINE_HEIGHT_MAX: f32 = 64.0;

/// パネルでの 1 つの変更。
#[derive(Clone, Debug, PartialEq)]
pub enum PanelEdit {
    Theme(ThemeId),
    Opacity(f32),
    Blur(bool),
    /// None はテーマの色に戻す。
    Accent(Option<u32>),
    /// None は OS の既定。
    UiFont(Option<String>),
    MonoFont(String),
    FontSize(u32),
    /// None は自動（大きさの 1.3 倍）。
    LineHeight(Option<f32>),
    CursorShape(CursorShape),
    CursorBlink(bool),
    /// 新しい版を確かめるか。
    UpdateCheck(bool),
}

/// 設定ファイルへの書き込み 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub enum FileEdit {
    Set(&'static [&'static str], EditValue),
    Remove(&'static [&'static str]),
}

/// 小数第 2 位までに丸める（設定ファイルに 0.800000011920929 のような値を書かないため）。
fn round2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

fn shape_name(shape: CursorShape) -> &'static str {
    match shape {
        CursorShape::Block => "block",
        CursorShape::Bar => "bar",
        CursorShape::Underline => "underline",
    }
}

impl PanelEdit {
    /// s に反映する。範囲の外の値は範囲に収める。
    pub fn apply(&self, s: &mut Settings) {
        match self {
            PanelEdit::Theme(id) => s.theme = *id,
            PanelEdit::Opacity(v) => s.window.background_opacity = round2(v.clamp(OPACITY_MIN, OPACITY_MAX)) as f32,
            PanelEdit::Blur(b) => s.window.blur = *b,
            PanelEdit::Accent(c) => s.accent = *c,
            PanelEdit::UiFont(f) => s.font.ui = f.clone().filter(|f| !f.is_empty()),
            PanelEdit::MonoFont(f) => s.font.mono = f.clone(),
            PanelEdit::FontSize(n) => {
                s.font.size = (*n).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX) as f32;
                if !s.font.line_height_explicit {
                    s.font.line_height = line_height_for(s.font.size);
                } else if s.font.line_height < s.font.size {
                    s.font.line_height = s.font.size;
                }
            }
            PanelEdit::LineHeight(Some(h)) => {
                s.font.line_height = (round2(*h) as f32).clamp(s.font.size, LINE_HEIGHT_MAX);
                s.font.line_height_explicit = true;
            }
            PanelEdit::LineHeight(None) => {
                s.font.line_height = line_height_for(s.font.size);
                s.font.line_height_explicit = false;
            }
            PanelEdit::CursorShape(shape) => s.cursor.shape = *shape,
            PanelEdit::CursorBlink(b) => s.cursor.blink = *b,
            PanelEdit::UpdateCheck(b) => s.updates.check = *b,
        }
    }

    /// apply したあとの s から、設定ファイルへの書き込みを作る。
    pub fn file_edits(&self, s: &Settings) -> Vec<FileEdit> {
        match self {
            PanelEdit::Theme(id) => vec![FileEdit::Set(&["theme"], EditValue::Str(id.name().to_string()))],
            PanelEdit::Opacity(_) => {
                vec![FileEdit::Set(&["window", "background_opacity"], EditValue::Float(round2(s.window.background_opacity)))]
            }
            PanelEdit::Blur(b) => vec![FileEdit::Set(&["window", "blur"], EditValue::Bool(*b))],
            PanelEdit::Accent(Some(c)) => vec![FileEdit::Set(&["accent"], EditValue::Str(format!("#{c:06x}")))],
            PanelEdit::Accent(None) => vec![FileEdit::Remove(&["accent"])],
            PanelEdit::UiFont(_) => vec![FileEdit::Set(&["font", "ui"], EditValue::Str(s.font.ui.clone().unwrap_or_default()))],
            PanelEdit::MonoFont(f) => vec![FileEdit::Set(&["font", "mono"], EditValue::Str(f.clone()))],
            PanelEdit::FontSize(_) => {
                let mut edits = vec![FileEdit::Set(&["font", "size"], EditValue::Int(s.font.size as i64))];
                if s.font.line_height_explicit {
                    edits.push(FileEdit::Set(&["font", "line_height"], EditValue::Float(round2(s.font.line_height))));
                }
                edits
            }
            PanelEdit::LineHeight(Some(_)) => {
                vec![FileEdit::Set(&["font", "line_height"], EditValue::Float(round2(s.font.line_height)))]
            }
            PanelEdit::LineHeight(None) => vec![FileEdit::Remove(&["font", "line_height"])],
            PanelEdit::CursorShape(shape) => vec![FileEdit::Set(&["cursor", "shape"], EditValue::Str(shape_name(*shape).to_string()))],
            PanelEdit::CursorBlink(b) => vec![FileEdit::Set(&["cursor", "blink"], EditValue::Bool(*b))],
            PanelEdit::UpdateCheck(b) => vec![FileEdit::Set(&["updates", "check"], EditValue::Bool(*b))],
        }
    }

    /// スライダーのように続けて来る変更か（ファイルへの書き込みをまとめる）。
    pub fn is_continuous(&self) -> bool {
        matches!(self, PanelEdit::Opacity(_))
    }
}

/// text に edit を書いた文面。
pub fn edit_text(text: &str, edit: &FileEdit) -> Result<String, EditError> {
    match edit {
        FileEdit::Set(path, value) => set_value(text, path, value.clone()),
        FileEdit::Remove(path) => remove_value(text, path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CursorShape, DEFAULT_CONFIG, ParseOutcome, Settings, ThemeId, line_height_for, parse};
    use crate::settings_edit::EditValue;

    fn applied(edit: &PanelEdit) -> Settings {
        let mut s = Settings::default();
        edit.apply(&mut s);
        s
    }

    #[test]
    fn panel_edits_map_to_file() {
        let s = applied(&PanelEdit::Opacity(0.8));
        assert_eq!(PanelEdit::Opacity(0.8).file_edits(&s), vec![FileEdit::Set(&["window", "background_opacity"], EditValue::Float(0.8))]);
        assert_eq!(applied(&PanelEdit::Opacity(0.1)).window.background_opacity, 0.3);
        assert_eq!(applied(&PanelEdit::FontSize(40)).font.size, 32.0);
        assert_eq!(PanelEdit::Accent(None).file_edits(&Settings::default()), vec![FileEdit::Remove(&["accent"])]);
        assert_eq!(
            PanelEdit::Accent(Some(0xff0080)).file_edits(&Settings::default()),
            vec![FileEdit::Set(&["accent"], EditValue::Str("#ff0080".into()))]
        );
        assert_eq!(PanelEdit::UiFont(None).file_edits(&Settings::default()), vec![FileEdit::Set(&["font", "ui"], EditValue::Str(String::new()))]);
        assert_eq!(PanelEdit::Theme(ThemeId::Dracula).file_edits(&Settings::default()), vec![FileEdit::Set(&["theme"], EditValue::Str("dracula".into()))]);
        assert_eq!(
            PanelEdit::CursorShape(CursorShape::Bar).file_edits(&Settings::default()),
            vec![FileEdit::Set(&["cursor", "shape"], EditValue::Str("bar".into()))]
        );
    }

    #[test]
    fn update_check_maps_to_file() {
        let s = applied(&PanelEdit::UpdateCheck(false));
        assert!(!s.updates.check);
        assert_eq!(PanelEdit::UpdateCheck(false).file_edits(&s), vec![FileEdit::Set(&["updates", "check"], EditValue::Bool(false))]);
    }

    #[test]
    fn line_height_auto_follows_size() {
        let mut s = Settings::default();
        PanelEdit::LineHeight(Some(22.0)).apply(&mut s);
        assert!(s.font.line_height_explicit);
        assert_eq!(s.font.line_height, 22.0);
        PanelEdit::LineHeight(None).apply(&mut s);
        assert!(!s.font.line_height_explicit);
        assert_eq!(s.font.line_height, line_height_for(s.font.size));
        assert_eq!(PanelEdit::LineHeight(None).file_edits(&s), vec![FileEdit::Remove(&["font", "line_height"])]);
        PanelEdit::FontSize(18).apply(&mut s);
        assert_eq!(s.font.line_height, line_height_for(18.0));
        assert_eq!(PanelEdit::FontSize(18).file_edits(&s), vec![FileEdit::Set(&["font", "size"], EditValue::Int(18))]);
    }

    #[test]
    fn bigger_font_raises_explicit_line_height() {
        let mut s = Settings::default();
        PanelEdit::LineHeight(Some(16.0)).apply(&mut s);
        PanelEdit::FontSize(20).apply(&mut s);
        assert!(s.font.line_height_explicit);
        assert_eq!(s.font.line_height, 20.0);
        assert_eq!(
            PanelEdit::FontSize(20).file_edits(&s),
            vec![FileEdit::Set(&["font", "size"], EditValue::Int(20)), FileEdit::Set(&["font", "line_height"], EditValue::Float(20.0))]
        );
    }

    #[test]
    fn only_opacity_is_continuous() {
        assert!(PanelEdit::Opacity(0.5).is_continuous());
        assert!(!PanelEdit::Blur(false).is_continuous());
        assert!(!PanelEdit::FontSize(12).is_continuous());
    }

    #[test]
    fn edit_text_roundtrip() {
        let edits = [
            PanelEdit::Theme(ThemeId::TokyoNight),
            PanelEdit::Opacity(0.85),
            PanelEdit::Blur(false),
            PanelEdit::Accent(Some(0x12ab34)),
            PanelEdit::Accent(None),
            PanelEdit::UiFont(Some("Segoe UI".into())),
            PanelEdit::UiFont(None),
            PanelEdit::MonoFont("Cascadia Code".into()),
            PanelEdit::FontSize(18),
            PanelEdit::LineHeight(Some(24.0)),
            PanelEdit::LineHeight(None),
            PanelEdit::CursorShape(CursorShape::Underline),
            PanelEdit::CursorBlink(true),
            PanelEdit::UpdateCheck(false),
        ];
        let mut text = DEFAULT_CONFIG.to_string();
        let mut want = Settings::default();
        for edit in edits {
            edit.apply(&mut want);
            for fe in edit.file_edits(&want) {
                text = edit_text(&text, &fe).unwrap();
            }
            let ParseOutcome::Ok { settings, issues } = parse(&text) else { panic!("{edit:?}: 読めない") };
            assert!(issues.is_empty(), "{edit:?}: {issues:?}");
            assert_eq!(settings, want, "{edit:?}");
        }
        assert!(text.contains("# accent ="), "コメントは残る");
    }
}
