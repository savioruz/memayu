use maud::html;

pub fn render_page(
    active_path: &str,
    email: Option<&str>,
    topbar_title: &str,
    title: &str,
    children: maud::Markup,
) -> String {
    let initials = email
        .and_then(|e| e.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();

    html! {
        (maud::DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - Memayu" }
                link href="/static/mu.slate.css" rel="stylesheet";
                link href="/static/memayu.css" rel="stylesheet";
                script src="/static/htmx.min.js" {}
                script defer src="/static/alpine.min.js" {}
                script {
                    (maud::PreEscaped("
                        (function() {
                            var saved = localStorage.getItem('memayu-theme');
                            if (saved) {
                                document.documentElement.setAttribute('data-theme', saved);
                            } else if (window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches) {
                                document.documentElement.setAttribute('data-theme', 'light');
                            } else {
                                document.documentElement.setAttribute('data-theme', 'dark');
                            }
                        })();
                    "))
                }
            }
            body class="min-h-screen bg-base-300" {
                div class="drawer lg:drawer-open" {
                    input id="drawer-toggle" type="checkbox" class="drawer-toggle";

                    // ── Drawer content ──
                    div class="drawer-content flex flex-col min-h-screen" {
                        // Top bar
                        header class="navbar bg-base-100 border-b border-base-200 px-2 lg:px-4 sticky top-0 z-10" {
                            div class="flex-none lg:hidden -ml-2" {
                                label for="drawer-toggle" aria-label="open sidebar" class="btn btn-ghost btn-square" {
                                    svg xmlns="http://www.w3.org/2000/svg" class="size-5" fill="none"
                                        viewBox="0 0 24 24" stroke="currentColor" {
                                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                              d="M4 6h16M4 12h16M4 18h7" {}
                                    }
                                }
                            }
                            div class="flex-1" {
                                h1 class="text-lg font-semibold" { (topbar_title) }
                            }
                            div class="flex-none flex items-center gap-1 -mr-2" {
                                button type="button" class="btn btn-ghost btn-circle" aria-label="Toggle theme" onclick="
                                    var current = document.documentElement.getAttribute('data-theme') === 'light' ? 'dark' : 'light';
                                    document.documentElement.setAttribute('data-theme', current);
                                    localStorage.setItem('memayu-theme', current);
                                " {
                                    svg xmlns="http://www.w3.org/2000/svg" class="size-5 theme-icon-sun" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z" {}
                                    }
                                    svg xmlns="http://www.w3.org/2000/svg" class="size-5 theme-icon-moon" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z" {}
                                    }
                                }
                                @if let Some(e) = email {
                                    div class="dropdown dropdown-end" {
                                        div tabindex="0" role="button"
                                            class="btn btn-ghost btn-circle avatar placeholder" {
                                            div class="bg-neutral text-neutral-content w-8" {
                                                span class="text-xs" { (initials) }
                                            }
                                        }
                                        ul tabindex="0"
                                            class="menu menu-sm dropdown-content bg-base-200 z-20 mt-3 w-52 p-2 shadow" {
                                            li { a class="text-xs opacity-60 pointer-events-none" { (e) } }
                                            div class="divider my-1" {}
                                            li { a href="/logout" { "Sign out" } }
                                        }
                                    }
                                }
                            }
                        }

                        main class="flex-1 px-2 py-4 lg:px-4 lg:py-6" {
                            (children)
                        }
                    }

                    // ── Drawer sidebar ──
                    div class="drawer-side z-20" {
                        label for="drawer-toggle" aria-label="close sidebar" class="drawer-overlay" {}
                        aside class="bg-base-200 min-h-full flex flex-col border-r border-base-300" {
                            // Brand
                            div class="px-6 py-5" {
                                a href="/home" class="brand-link text-xl font-bold tracking-tight" { "Memayu" }
                                p class="text-xs text-base-content/50 mt-0.5" { "Memory Dashboard" }
                            }

                            ul class="menu px-3 flex-1 gap-0.5" {
                                (nav_item(active_path, "home", "Home", "/home", icon_home()))
                                (nav_item(active_path, "requests", "Requests", "/requests", icon_requests()))
                                div class="divider my-2 mx-2 opacity-25" {}
                                (nav_item(active_path, "api-keys", "API Keys", "/api-keys", icon_keys()))
                                (nav_item(active_path, "providers", "Configuration", "/providers", icon_config()))
                            }

                            div class="px-6 py-4 text-xs text-base-content/40 flex flex-col gap-1" {
                                a href="/docs" class="link link-hover" { "Docs" }
                                (format!("Memayu v{}", env!("CARGO_PKG_VERSION")))
                            }
                        }
                    }
                }
            }
        }
    }.into_string()
}

fn nav_item(active: &str, path: &str, label: &str, href: &str, icon: maud::Markup) -> maud::Markup {
    let is_active = active == path;
    html! {
        li {
            a href=(href) class=(if is_active { "menu-active font-small" } else { "" }) {
                span class="size-5 shrink-0" { (icon) }
                (label)
            }
        }
    }
}

// ── Icons ──

fn icon_home() -> maud::Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
            fill="none" stroke="currentColor"
            stroke-linejoin="round" stroke-linecap="round" stroke-width="2"
            class="size-5 inline mr-3" {
            path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8" {}
            path d="M3 10a2 2 0 0 1 .709-1.528l7-5.999a2 2 0 0 1 2.582 0l7 5.999A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" {}
        }
    }
}

fn icon_requests() -> maud::Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
            fill="none" stroke="currentColor"
            stroke-linejoin="round" stroke-linecap="round" stroke-width="2"
            class="size-5 inline mr-3" {
            path d="M3 3v18h18" {}
            path d="m19 9-5 5-4-4-3 3" {}
        }
    }
}

fn icon_keys() -> maud::Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
            fill="none" stroke="currentColor"
            stroke-linejoin="round" stroke-linecap="round" stroke-width="2"
            class="size-5 inline mr-3" {
            path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4" {}
        }
    }
}

fn icon_config() -> maud::Markup {
    html! {
        svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"
            fill="none" stroke="currentColor"
            stroke-linejoin="round" stroke-linecap="round" stroke-width="2"
            class="size-5 inline mr-3" {
            circle cx="12" cy="12" r="3" {}
            path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" {}
        }
    }
}
