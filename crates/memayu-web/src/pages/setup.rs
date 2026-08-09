use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use memayu_api::{auth_dto::SetupRequest, WebServices, SESSION_COOKIE, SESSION_DURATION_SECS};

// ── helpers ──

fn base_page(title: &str, content: maud::Markup) -> String {
    maud::html! {
        (maud::DOCTYPE)
        html lang="en" data-theme="dark" {
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
            body class="min-h-screen bg-base-300 flex items-center justify-center" {
                main class="w-full max-w-md px-4" {
                    (content)
                }
            }
        }
    }
    .into_string()
}

fn setup_form(email: &str, error: Option<&str>) -> maud::Markup {
    maud::html! {
        div class="card bg-base-100 shadow-xl" {
            div class="card-body" {
                h2 class="card-title text-2xl font-bold justify-center mb-4" { "Create Admin Account" }
                @if let Some(e) = error {
                    div class="alert alert-error mb-4" role="alert" x-data="{ open: true }" x-show="open" {
                        svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" {}
                        }
                        span class="flex-1" { (e) }
                        button type="button" class="alert-close" x-on:click="open = false" aria-label="Close alert" {
                            svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                                path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                            }
                        }
                    }
                }
                form method="post" action="/setup" class="space-y-4" {
                    fieldset class="fieldset" {
                        label class="label" { span { "Email" } }
                        input type="email" name="email" class="input w-full" placeholder="admin@example.com" value=(email) required;
                    }
                    fieldset class="fieldset" {
                        label class="label" { span { "Password" } }
                        input type="password" name="password" class="input w-full" placeholder="Min 8 chars, upper, lower, digit" required;
                    }
                    fieldset class="fieldset" {
                        label class="label" { span { "Confirm Password" } }
                        input type="password" name="confirm" class="input w-full" placeholder="••••••••" required;
                    }
                    button type="submit" class="btn btn-primary w-full mt-2" { "Create Account" }
                }
            }
        }
    }
}

// ── DTOs ──

#[derive(serde::Deserialize)]
pub struct SetupForm {
    pub email: String,
    pub password: String,
    pub confirm: String,
}

// ── handlers ──

pub async fn get_setup(
    State(services): State<WebServices>,
) -> Result<Html<String>, (StatusCode, String)> {
    let empty = services
        .auth_users_empty()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !empty {
        return Ok(Html(base_page(
            "Setup",
            maud::html! {
                div class="card bg-base-100 shadow-xl" {
                    div class="card-body text-center" {
                        h2 class="card-title justify-center" { "Already Set Up" }
                        p { "Setup has already been completed." }
                        a href="/login" class="btn btn-primary mt-4" { "Go to Login" }
                    }
                }
            },
        )));
    }
    Ok(Html(base_page("Setup", setup_form("", None))))
}

pub async fn post_setup(
    State(services): State<WebServices>,
    Form(form): Form<SetupForm>,
) -> Result<Response, (StatusCode, String)> {
    let req = SetupRequest {
        email: form.email,
        password: form.password,
        confirm: form.confirm,
    };

    match services.auth_setup(&req).await {
        Ok((_auth_response, token)) => {
            let mut resp = Redirect::to("/home").into_response();
            resp.headers_mut().insert(
                SET_COOKIE,
                format!(
                    "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_DURATION_SECS}",
                )
                .parse()
                .unwrap(),
            );
            Ok(resp)
        }
        Err(e) => {
            // Show the form again with the error.
            // 409 conflict means setup already done → redirect.
            if e.status == 409 {
                Ok(Redirect::to("/login").into_response())
            } else {
                Ok(Html(base_page(
                    "Setup",
                    setup_form(req.email.trim(), Some(&e.message)),
                ))
                .into_response())
            }
        }
    }
}
