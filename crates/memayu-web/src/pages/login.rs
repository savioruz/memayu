use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use memayu_api::{auth_dto::LoginRequest, WebServices, SESSION_COOKIE, SESSION_DURATION_SECS};

#[derive(serde::Deserialize)]
pub struct LoginForm {
    pub email: String,
    pub password: String,
}

fn login_card_body(email: &str, error: Option<&str>) -> maud::Markup {
    maud::html! {
        div id="login-card-body" class="card-body" {
            @if let Some(e) = error {
                div class="alert alert-error mb-4" role="alert" x-data="{ open: true }" x-show="open" {
                    span class="flex-1" { (e) }
                    button type="button" class="alert-close" x-on:click="open = false" aria-label="Close alert" {
                        svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                        }
                    }
                }
            }
            h2 class="card-title text-2xl font-bold justify-center mb-4" { "Log In" }
            form method="post" action="/login"
                 hx-post="/login" hx-target="#login-card-body" hx-swap="outerHTML" class="space-y-4" {
                fieldset class="fieldset" {
                    label class="label" { span { "Email" } }
                    input type="email" name="email" class="input w-full"
                        placeholder="admin@example.com" value=(email) required;
                }
                fieldset class="fieldset" {
                    label class="label" { span { "Password" } }
                    input type="password" name="password" class="input w-full"
                        placeholder="••••••••" required;
                }
                button type="submit" class="btn btn-primary w-full mt-2" {
                    "Log In"
                }
            }
        }
    }
}

fn login_page(email: &str, error: Option<&str>) -> String {
    maud::html! {
        (maud::DOCTYPE)
        html lang="en" data-theme="dark" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Login - Memayu" }
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
            body class="min-h-screen bg-base-300 flex items-center justify-center" {
                main class="w-full max-w-md px-4" {
                    div class="card bg-base-100 shadow-xl" {
                        (login_card_body(email, error))
                    }
                }
            }
        }
    }.into_string()
}

pub async fn get_login(
    State(services): State<WebServices>,
) -> Result<Html<String>, (StatusCode, String)> {
    if services
        .auth_users_empty()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Ok(Html(maud::html! {
            (maud::DOCTYPE)
            html lang="en" data-theme="dark" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    title { "Welcome - Memayu" }
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
                body class="min-h-screen bg-base-300 flex items-center justify-center" {
                    main class="w-full max-w-md px-4" {
                        div class="card bg-base-100 shadow-xl" {
                            div class="card-body text-center" {
                                h2 class="card-title justify-center" { "Welcome" }
                                p { "No accounts found. Create your admin account first." }
                                a href="/setup" class="btn btn-primary mt-4" { "Create Account" }
                            }
                        }
                    }
                }
            }
        }.into_string()));
    }
    Ok(Html(login_page("", None)))
}

pub async fn post_login(
    State(services): State<WebServices>,
    headers: axum::http::HeaderMap,
    Form(form): Form<LoginForm>,
) -> Result<Response, (StatusCode, String)> {
    let req = LoginRequest {
        email: form.email,
        password: form.password,
    };

    match services.auth_login(&req).await {
        Ok((_auth_response, token)) => {
            let is_htmx = headers
                .get("hx-request")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("true"));

            let cookie = format!(
                "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_DURATION_SECS}",
            );

            if is_htmx {
                // htmx would follow a 303 and swap the whole /home document into the login
                // card. Return an empty response with HX-Redirect so htmx navigates instead.
                let mut resp = StatusCode::NO_CONTENT.into_response();
                resp.headers_mut()
                    .insert(SET_COOKIE, cookie.parse().unwrap());
                resp.headers_mut().insert(
                    axum::http::header::HeaderName::from_static("hx-redirect"),
                    axum::http::HeaderValue::from_static("/home"),
                );
                Ok(resp)
            } else {
                let mut resp = Redirect::to("/home").into_response();
                resp.headers_mut()
                    .insert(SET_COOKIE, cookie.parse().unwrap());
                Ok(resp)
            }
        }
        Err(e) => {
            if e.status == 401 {
                Ok(Html(
                    login_card_body(req.email.trim(), Some("Invalid credentials.")).into_string(),
                )
                .into_response())
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, e.message))
            }
        }
    }
}
