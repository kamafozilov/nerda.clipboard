//! How the shelf looks: a glass panel along the bottom of the screen with a search field,
//! kind filters and a row of cards.

use rune::{Color, Cx, El, Family, Icon, Spring, SurfaceFit, TextStyle, col, el, icon, row, spacer, text, text_input};

use crate::model::{Entry, Filter, Kind, ago, link_host};
use crate::ui::{self, CARD_PITCH, CARD_W, MARGIN, PANEL_H, Phase, STRIP_PAD, Ui, card_key, strip_width};

const RADIUS: f32 = 26.0;
const HEADER_H: f32 = 52.0;
const FOOTER_H: f32 = 40.0;
pub const CARD_H: f32 = PANEL_H - HEADER_H - FOOTER_H - 12.0;
/// How far below its place the panel starts when it opens.
const RISE: f32 = 36.0;
const FILE_ROWS: usize = 6;

static SEARCH: Icon = Icon {
    name: "nerda-search",
    view: 24.0,
    body: r#"<circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="currentColor" stroke-width="2.2"/><path d="M15.5 15.5 L20.5 20.5" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>"#,
};
static PIN: Icon = Icon {
    name: "nerda-pin",
    view: 24.0,
    body: r#"<path d="M9 3 H15 L14.2 9.6 L18 13.4 V15 H12.9 V21 L12 22 L11.1 21 V15 H6 V13.4 L9.8 9.6 Z" fill="currentColor"/>"#,
};
static FILE: Icon = Icon {
    name: "nerda-file",
    view: 24.0,
    body: r#"<path d="M6.5 3 H13.5 L18.5 8 V21 H6.5 Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="M13.5 3 V8 H18.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>"#,
};

/// The colours of one appearance.
#[derive(Clone, Copy)]
struct Look {
    text: Color,
    dim: Color,
    faint: Color,
    card: Color,
    card_on: Color,
    hover: Color,
    stroke: Color,
    well: Color,
    panel: Color,
    panel_edge: Color,
    keycap: Color,
    shadow: Color,
    accent: Color,
    warning: Color,
}

impl Look {
    fn new(dark: bool, accent: Color) -> Look {
        let (white, black) = (Color::WHITE, Color::BLACK);
        let hex = |s: &str| rune::color::hex(s).unwrap_or(Color::WHITE);
        if dark {
            Look {
                text: white.alpha(0.92),
                dim: white.alpha(0.6),
                faint: white.alpha(0.38),
                card: white.alpha(0.075),
                card_on: white.alpha(0.15),
                hover: white.alpha(0.05),
                stroke: white.alpha(0.1),
                well: black.alpha(0.28),
                panel: hex("#1d1d20").alpha(0.86),
                panel_edge: white.alpha(0.16),
                keycap: white.alpha(0.11),
                shadow: black.alpha(0.42),
                accent,
                warning: hex("#ffb340"),
            }
        } else {
            Look {
                text: black.alpha(0.88),
                dim: black.alpha(0.56),
                faint: black.alpha(0.36),
                card: white.alpha(0.58),
                card_on: white.alpha(0.92),
                hover: black.alpha(0.03),
                stroke: black.alpha(0.08),
                well: black.alpha(0.06),
                panel: hex("#f3f3f5").alpha(0.88),
                panel_edge: white.alpha(0.7),
                keycap: black.alpha(0.07),
                shadow: black.alpha(0.24),
                accent,
                warning: hex("#b25e00"),
            }
        }
    }
}

const fn font(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let l = Look::new(cx.system_dark(), cx.accent());
    let listed = s.listed();
    let shown = s.phase == Phase::Shown;

    let body = if listed.is_empty() { empty(s, &l) } else { strip(s, &l, &listed) };
    let panel = col()
        .key("panel")
        .w_full()
        .h(PANEL_H)
        .radius(RADIUS)
        .fill(l.panel)
        .border(1.0, l.panel_edge)
        .shadow(14.0, 40.0, l.shadow)
        .system_material(s.material)
        .offset(0.0, if shown { 0.0 } else { RISE })
        .offset_motion(Spring::new(0.34, 0.12))
        .opacity(if shown { 1.0 } else { 0.0 })
        .opacity_motion(Spring::new(0.16, 0.0))
        .child(header(s, &l))
        .child(body)
        .child(footer(s, &l, listed.len()));

    let root = col().key("root").w_full().h_full().px(MARGIN).pb(MARGIN).justify_end().child(panel);
    // A headless render has no desktop behind the window: stand a wallpaper in for it.
    if s.demo {
        let hex = |s: &str| rune::color::hex(s).unwrap_or(Color::BLACK);
        let (top, bottom) = if cx.system_dark() { (hex("#16213e"), hex("#5b2a6e")) } else { (hex("#8ec5fc"), hex("#e0c3fc")) };
        root.gradient(top, bottom)
    } else {
        root
    }
}

fn header(s: &Ui, l: &Look) -> El {
    let field = text_input(&s.query, font(15.0, 400.0).lh(22.0))
        .key("query")
        .single_line()
        .grow(1.0)
        .color(l.text)
        .placeholder("Search clipboard history", l.faint)
        .caret(if s.caret_on { l.accent } else { l.accent.alpha(0.0) }, l.accent.alpha(0.3))
        .on_edit(|s: &mut Ui| &mut s.query)
        .on_input_key(ui::on_key)
        .on_input_change(|s: &mut Ui, tx| s.rewind(tx));

    let mut filters = row().key("filters").gap(2.0).center_y();
    for filter in Filter::ALL {
        let on = s.filter == filter;
        filters = filters.child(
            row()
                .key(format!("filter-{}", filter.label()))
                .role("button")
                .label(filter.label())
                .h(26.0)
                .px(11.0)
                .radius(13.0)
                .center()
                .fill(if on { l.accent } else { Color::TRANSPARENT })
                .hover(l.hover)
                .preserve_focus_on_press(true)
                .on_press_tx(move |s: &mut Ui, tx| ui::set_filter(s, filter, tx))
                .child(text(filter.label()).font(font(12.0, 500.0)).color(if on { Color::WHITE } else { l.dim })),
        );
    }

    row()
        .key("header")
        .w_full()
        .h(HEADER_H)
        .px(18.0)
        .gap(10.0)
        .center_y()
        .child(icon(&SEARCH, 16.0).color(l.dim))
        .child(field)
        .child(filters)
}

fn strip(s: &Ui, l: &Look, listed: &[usize]) -> El {
    let range = s.mounted_now(listed.len());
    let mut cards = el().key("cards").w(strip_width(listed.len())).h_full().on_layout_tx(|s: &mut Ui, rect, tx| ui::strip_moved(s, rect[0], tx));
    for position in range {
        let x = STRIP_PAD + position as f32 * CARD_PITCH;
        cards = cards.child(card(s, l, &s.entries[listed[position]], position).absolute(x, 6.0));
    }
    let scroll = s.scroll.clone();
    el().key("strip")
        .w_full()
        .h(CARD_H + 12.0)
        .scroll_x()
        .scroll_edge_fade(18.0)
        .on_scroll_offset(move |at| scroll.set([at.offset[0], at.viewport[0]]))
        .child(cards)
}

fn empty(s: &Ui, l: &Look) -> El {
    let query = s.query.text.trim();
    let (title, hint) = if !query.is_empty() {
        (format!("No matches for “{query}”"), "Press esc to clear the search.".to_owned())
    } else if s.filter != Filter::All {
        (format!("Nothing under {}", s.filter.label()), "Press ⇥ to switch filters.".to_owned())
    } else {
        ("Nothing copied yet".to_owned(), format!("Copy something, then press {} to find it here.", s.shortcut))
    };
    col()
        .key("empty")
        .w_full()
        .h(CARD_H + 12.0)
        .center()
        .gap(6.0)
        .child(text(title).font(font(15.0, 600.0)).color(l.text))
        .child(text(hint).font(font(12.5, 400.0)).color(l.dim))
}

fn card(s: &Ui, l: &Look, e: &Entry, position: usize) -> El {
    let on = position == s.selected;
    let id = e.id;
    let thumb = s.thumbs.get(&id);

    let mut head = row().key("head").w_full().h(34.0).px(12.0).gap(7.0).center_y();
    if let Some(app_icon) = s.icons.get(&e.app_path).filter(|image| image.error().is_none()) {
        head = head.child(el().size(16.0, 16.0).pixels(app_icon.pixels()));
    }
    let source = if e.app_name.is_empty() { e.kind.label() } else { e.app_name.as_str() };
    head = head.child(text(source).font(font(11.5, 600.0)).color(l.dim).truncate().grow(1.0).min_w(0.0));
    if e.pinned {
        head = head.child(icon(&PIN, 12.0).color(l.accent));
    }
    head = head.child(text(ago(s.now, e.used)).font(font(11.0, 500.0)).color(l.faint));

    let body = match e.kind {
        _ if e.thumb.is_some() => {
            let mut picture = el().key("picture").w_full().grow(1.0).min_h(0.0).radius(9.0).fill(l.well);
            if let Some(image) = thumb {
                picture = picture.pixels_fit(image.pixels(), SurfaceFit::Fill);
            }
            col().w_full().grow(1.0).min_h(0.0).px(8.0).child(picture)
        }
        Kind::Text | Kind::Image => col().w_full().grow(1.0).min_h(0.0).px(12.0).child(
            text(e.preview.as_str()).font(font(12.5, 400.0).lh(17.0)).color(l.text).w_full().wrap().max_lines(8),
        ),
        Kind::Link => col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(12.0)
            .gap(5.0)
            .child(text(link_host(&e.preview)).font(font(15.0, 600.0).lh(20.0)).color(l.text).w_full().truncate())
            .child(text(e.preview.as_str()).font(font(11.5, 400.0).lh(15.5)).color(l.dim).w_full().wrap().wrap_anywhere().max_lines(6)),
        Kind::Color => {
            let [r, g, b, a] = e.color.unwrap_or([0, 0, 0, 255]);
            let swatch = rune::color::hex(&format!("#{r:02x}{g:02x}{b:02x}{a:02x}")).unwrap_or(Color::BLACK);
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(12.0)
                .gap(8.0)
                .child(el().key("swatch").w_full().grow(1.0).min_h(0.0).radius(10.0).fill(swatch).border(1.0, l.stroke))
                .child(text(e.preview.as_str()).font(font(13.0, 600.0).family(Family::SystemMono)).color(l.text).truncate())
        }
        Kind::Files => {
            let names: Vec<&str> = e.preview.lines().collect();
            let mut list = col().w_full().grow(1.0).min_h(0.0).px(12.0).gap(5.0);
            for name in names.iter().take(FILE_ROWS) {
                list = list.child(
                    row()
                        .w_full()
                        .gap(6.0)
                        .center_y()
                        .child(icon(&FILE, 14.0).color(l.dim))
                        .child(text(*name).font(font(12.5, 400.0)).color(l.text).truncate().grow(1.0).min_w(0.0)),
                );
            }
            if names.len() > FILE_ROWS {
                list = list.child(text(format!("and {} more", names.len() - FILE_ROWS)).font(font(11.5, 400.0)).color(l.dim));
            }
            list
        }
    };

    // A file shown as a picture is named under it.
    let caption = if e.kind == Kind::Files && e.thumb.is_some() { format!("{} · {}", e.preview, e.detail) } else { e.detail.clone() };
    let mut foot = row()
        .key("foot")
        .w_full()
        .h(32.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .child(text(caption).font(font(11.0, 400.0)).color(l.dim).truncate().grow(1.0).min_w(0.0));
    if position < 9 {
        foot = foot.child(keycap(l, &format!("⌘{}", position + 1)));
    }

    col()
        .key(card_key(id))
        .role("listitem")
        .label(format!("{}: {}", e.kind.label(), e.preview.lines().next().unwrap_or("")))
        .size(CARD_W, CARD_H)
        .radius(16.0)
        .continuous_corners()
        .fill(if on { l.card_on } else { l.card })
        .border(if on { 2.0 } else { 1.0 }, if on { l.accent } else { l.stroke })
        .hover(l.hover)
        .preserve_focus_on_press(true)
        .on_press_tx(move |s: &mut Ui, tx| ui::select_card(s, id, tx))
        .on_double_press_tx(move |s: &mut Ui, tx| ui::paste_card(s, id, tx))
        // A card coming into view is when the pictures around it are worth loading.
        .on_visible(|s: &mut Ui, _, _| s.sync_images())
        .child(head)
        .child(body)
        .child(foot)
}

fn keycap(l: &Look, label: &str) -> El {
    row().h(18.0).px(5.0).radius(5.0).center().fill(l.keycap).child(text(label).font(font(10.5, 500.0)).color(l.dim))
}

fn footer(s: &Ui, l: &Look, count: usize) -> El {
    let mut bar = row().key("footer").w_full().h(FOOTER_H).px(18.0).gap(14.0).center_y();
    if let Some(notice) = &s.notice {
        bar = bar.child(text(notice.as_str()).key("notice").font(font(11.5, 500.0)).color(l.warning).truncate().grow(1.0).min_w(0.0));
    } else {
        for (keys, action) in [("↩", "Paste"), ("⇧↩", "Paste plain"), ("⌘C", "Copy"), ("⌘P", "Pin"), ("⌘⌫", "Delete"), ("⇥", "Filter"), ("esc", "Close")] {
            bar = bar.child(row().gap(5.0).center_y().child(keycap(l, keys)).child(text(action).font(font(11.5, 400.0)).color(l.dim)));
        }
        bar = bar.child(spacer());
    }
    let items = if count == 1 { "1 item".to_owned() } else { format!("{count} items") };
    let status = if s.paused { format!("{items} · Paused") } else { items };
    bar.child(text(status).key("count").font(font(11.5, 400.0)).color(if s.paused { l.warning } else { l.faint }))
}
