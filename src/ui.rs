//! Gemeinsames Aussehen: Theme, Karten, Badges und Formatierungshilfen.
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(88, 142, 255);
pub const OK: Color32 = Color32::from_rgb(82, 196, 140);
pub const WARN: Color32 = Color32::from_rgb(240, 180, 70);
pub const ERR: Color32 = Color32::from_rgb(240, 100, 100);
pub const BG: Color32 = Color32::from_rgb(24, 26, 31);
pub const SIDEBAR: Color32 = Color32::from_rgb(19, 21, 25);
pub const CARD: Color32 = Color32::from_rgb(33, 36, 43);
pub const BORDER: Color32 = Color32::from_rgb(48, 52, 61);
pub const MUTED: Color32 = Color32::from_rgb(140, 147, 160);

/// Dunkles Theme mit Akzentfarbe, größeren Abständen und runden Ecken.
pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = Color32::from_rgb(17, 19, 23);
        v.faint_bg_color = Color32::from_rgb(29, 32, 38);
        v.selection.bg_fill = ACCENT.linear_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        let r = CornerRadius::same(8);
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = r;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(45, 49, 59);
        v.widgets.inactive.bg_fill = Color32::from_rgb(45, 49, 59);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(58, 64, 77);
        v.widgets.hovered.bg_fill = Color32::from_rgb(58, 64, 77);
        v.widgets.active.weak_bg_fill = ACCENT.linear_multiply(0.7);
        v.widgets.active.bg_fill = ACCENT.linear_multiply(0.7);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.scroll.bar_width = 8.0;
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.5));
    });
}

/// Abgerundete Karte mit optionaler Überschrift.
pub fn card<R>(ui: &mut egui::Ui, title: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(t) = title {
                ui.label(RichText::new(t).strong().size(16.0));
                ui.add_space(2.0);
            }
            add(ui)
        })
        .inner
}

/// Hervorgehobener Hauptknopf.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).strong().color(Color32::WHITE)).fill(ACCENT).min_size(egui::vec2(0.0, 36.0))
}

/// Kleines farbiges Etikett.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.linear_multiply(0.18))
        .corner_radius(CornerRadius::same(20))
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).strong().size(13.0));
        });
}

/// Dezenter Hilfetext.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED).size(13.0));
}

/// Kurzer Hinweis mit aufklappbarer Langfassung („ⓘ Mehr“).
pub fn hint_long(ui: &mut egui::Ui, id: &str, short: &str, long: &str) {
    ui.horizontal_wrapped(|ui| {
        hint(ui, short);
    });
    egui::CollapsingHeader::new(RichText::new("ⓘ Mehr").color(MUTED).size(13.0)).id_salt(id).show(ui, |ui| {
        hint(ui, long);
    });
}

/// Farbe passend zum Inhalt einer Statusmeldung.
pub fn status_color(msg: &str) -> Color32 {
    if msg.starts_with("FEHLER") || msg.contains("falsch") || msg.contains("kein Repository") {
        ERR
    } else if msg.starts_with("Warnung") {
        WARN
    } else {
        OK
    }
}

/// Statusmeldung farbig anzeigen (nichts bei leerem Text).
pub fn status(ui: &mut egui::Ui, msg: &str) {
    if !msg.is_empty() {
        ui.label(RichText::new(msg).color(status_color(msg)));
    }
}

/// Zahl von Bytes lesbar formatieren (1024er-Schritte).
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

/// ISO-Zeitstempel von restic (`2026-11-05T02:00:01.123+01:00`) als `05.11.2026 02:00`.
pub fn short_time(iso: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(t) => t.with_timezone(&chrono::Local).format("%d.%m.%Y %H:%M").to_string(),
        Err(_) => iso.chars().take(16).collect::<String>().replace('T', " "),
    }
}

/// Zeitraum-Filter für die Snapshot-Liste.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SnapRange {
    All,
    Days7,
    Days30,
    Year,
}

impl SnapRange {
    pub const ALL: [(SnapRange, &'static str); 4] = [(SnapRange::All, "Alle"), (SnapRange::Days7, "7 Tage"), (SnapRange::Days30, "30 Tage"), (SnapRange::Year, "1 Jahr")];

    fn days(self) -> Option<i64> {
        match self {
            SnapRange::All => None,
            SnapRange::Days7 => Some(7),
            SnapRange::Days30 => Some(30),
            SnapRange::Year => Some(365),
        }
    }
}

/// Passt ein Snapshot zu Textfilter, Ordner und Zeitraum? Alle Filter wirken zusammen (UND).
pub fn snapshot_matches(s: &crate::restic::Snapshot, needle: &str, folder: Option<&str>, range: SnapRange, now: chrono::DateTime<chrono::Local>) -> bool {
    if let Some(f) = folder {
        if !s.paths.iter().any(|p| p == f) {
            return false;
        }
    }
    if let (Some(days), Ok(t)) = (range.days(), chrono::DateTime::parse_from_rfc3339(&s.time)) {
        if now.signed_duration_since(t).num_days() >= days {
            return false;
        }
    }
    let needle = needle.to_lowercase();
    needle.is_empty() || [short_time(&s.time), s.hostname.clone(), s.paths.join(", "), s.short_id.clone()].iter().any(|f| f.to_lowercase().contains(&needle))
}

/// Alter des letzten Laufs aus der Kopfzeile des Protokolls (`2026-11-05 08:39:31 – ERFOLGREICH`)
/// als Text und Hinweis, ob er überfällig ist (älter als 2 Tage).
pub fn age_text(head: &str, now: chrono::NaiveDateTime) -> Option<(String, bool)> {
    let ts = chrono::NaiveDateTime::parse_from_str(head.get(..19)?, "%Y-%m-%d %H:%M:%S").ok()?;
    let d = now.signed_duration_since(ts);
    let text = if d.num_minutes() < 1 {
        "gerade eben".to_string()
    } else if d.num_hours() < 1 {
        format!("vor {} Min.", d.num_minutes())
    } else if d.num_hours() < 48 {
        format!("vor {} Std.", d.num_hours())
    } else {
        format!("vor {} Tagen", d.num_days())
    };
    Some((text, d.num_hours() >= 48))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_lesbar() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn zeit_kurz() {
        assert_eq!(short_time("kaputt-und-lang-genug-zum-kuerzen"), "kaputt-und-lang-");
        assert!(short_time("2026-11-05T02:00:01+00:00").contains("2026"));
    }

    #[test]
    fn alter_des_laufs() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(age_text("2026-11-05 11:30:00 – ERFOLGREICH", now), Some(("vor 30 Min.".into(), false)));
        assert_eq!(age_text("2026-11-04 12:00:00 – FEHLER", now), Some(("vor 24 Std.".into(), false)));
        assert_eq!(age_text("2026-11-02 12:00:00 – ERFOLGREICH", now), Some(("vor 3 Tagen".into(), true)));
        assert_eq!(age_text("kaputt", now), None);
    }

    fn snap(time: &str, paths: &[&str]) -> crate::restic::Snapshot {
        crate::restic::Snapshot { id: "abc123".into(), short_id: "abc123".into(), time: time.into(), hostname: "pc".into(), paths: paths.iter().map(|p| p.to_string()).collect() }
    }

    #[test]
    fn snapshot_filter() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-05T12:00:00+00:00").unwrap().with_timezone(&chrono::Local);
        let neu = snap("2026-11-04T12:00:00+00:00", &["/a"]);
        let alt = snap("2026-08-01T12:00:00+00:00", &["/b"]);
        assert!(snapshot_matches(&neu, "", None, SnapRange::All, now));
        assert!(snapshot_matches(&neu, "", None, SnapRange::Days7, now));
        assert!(!snapshot_matches(&alt, "", None, SnapRange::Days30, now));
        assert!(snapshot_matches(&alt, "", None, SnapRange::Year, now));
        assert!(snapshot_matches(&alt, "", Some("/b"), SnapRange::All, now));
        assert!(!snapshot_matches(&alt, "", Some("/a"), SnapRange::All, now));
        assert!(snapshot_matches(&neu, "PC", None, SnapRange::All, now));
        assert!(!snapshot_matches(&neu, "xyz", None, SnapRange::All, now));
        // kombiniert: Ordner passt, Text nicht
        assert!(!snapshot_matches(&neu, "xyz", Some("/a"), SnapRange::All, now));
    }

    #[test]
    fn statusfarben() {
        assert_eq!(status_color("FEHLER: x"), ERR);
        assert_eq!(status_color("Gespeichert."), OK);
    }
}
