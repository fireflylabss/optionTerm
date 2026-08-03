//! Embedded browser tab backed by WebKitGTK 6.

use gtk4::{Button, Entry, Orientation, prelude::*};
use webkit6::{LoadEvent, WebView, prelude::WebViewExt};

const HOME_URI: &str = "https://duckduckgo.com/";

/// Build a browser surface that can live as an ordinary `AdwTabView` page.
pub fn new_tab() -> gtk4::Box {
    let back = icon_button("go-previous-symbolic", "Go back");
    let forward = icon_button("go-next-symbolic", "Go forward");
    let reload = icon_button("view-refresh-symbolic", "Reload");
    back.set_sensitive(false);
    forward.set_sensitive(false);

    let address = Entry::builder()
        .text(HOME_URI)
        .placeholder_text("Enter a web address")
        .hexpand(true)
        .activates_default(true)
        .build();
    address.add_css_class("browser-address");

    // This is a compact Adwaita toolbar, not another HeaderBar: the app
    // already owns the window chrome and tab strip above this page.
    let navigation = gtk4::Box::new(Orientation::Horizontal, 0);
    navigation.add_css_class("linked");
    navigation.append(&back);
    navigation.append(&forward);

    let toolbar = gtk4::Box::new(Orientation::Horizontal, 6);
    toolbar.add_css_class("toolbar");
    toolbar.add_css_class("flat");
    toolbar.set_margin_start(6);
    toolbar.set_margin_end(6);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);
    toolbar.append(&navigation);
    toolbar.append(&address);
    toolbar.append(&reload);

    let webview = WebView::new();
    webview.set_hexpand(true);
    webview.set_vexpand(true);

    let content = gtk4::Box::new(Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.append(&toolbar);
    content.append(&webview);

    {
        let webview = webview.downgrade();
        back.connect_clicked(move |_| {
            if let Some(webview) = webview.upgrade() {
                webview.go_back();
            }
        });
    }
    {
        let webview = webview.downgrade();
        forward.connect_clicked(move |_| {
            if let Some(webview) = webview.upgrade() {
                webview.go_forward();
            }
        });
    }
    {
        let webview = webview.downgrade();
        reload.connect_clicked(move |_| {
            if let Some(webview) = webview.upgrade() {
                webview.reload();
            }
        });
    }
    {
        let webview = webview.downgrade();
        address.connect_activate(move |entry| {
            let Some(webview) = webview.upgrade() else {
                return;
            };
            if let Some(uri) = normalize_address(&entry.text()) {
                webview.load_uri(&uri);
            }
        });
    }
    {
        let address = address.clone();
        let back = back.clone();
        let forward = forward.clone();
        webview.connect_load_changed(move |webview, event| {
            if matches!(event, LoadEvent::Committed | LoadEvent::Finished)
                && let Some(uri) = webview.uri()
            {
                address.set_text(uri.as_str());
            }
            back.set_sensitive(webview.can_go_back());
            forward.set_sensitive(webview.can_go_forward());
        });
    }

    webview.load_uri(HOME_URI);
    content
}

fn icon_button(icon_name: &str, tooltip: &str) -> Button {
    let button = Button::from_icon_name(icon_name);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button
}

fn normalize_address(address: &str) -> Option<String> {
    let address = address.trim();
    if address.is_empty() {
        return None;
    }
    if address.starts_with("http://")
        || address.starts_with("https://")
        || address.starts_with("about:")
    {
        Some(address.to_string())
    } else {
        Some(format!("https://{address}"))
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_address;

    #[test]
    fn normalizes_bare_web_addresses() {
        assert_eq!(
            normalize_address("example.com"),
            Some("https://example.com".into())
        );
        assert_eq!(
            normalize_address("  https://example.com  "),
            Some("https://example.com".into())
        );
    }

    #[test]
    fn preserves_supported_schemes_and_rejects_empty_input() {
        assert_eq!(
            normalize_address("http://localhost:3000"),
            Some("http://localhost:3000".into())
        );
        assert_eq!(normalize_address("about:blank"), Some("about:blank".into()));
        assert_eq!(normalize_address("   "), None);
    }
}
