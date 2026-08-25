//! Account settings page: change the logged-in admin's password and email.
//! Follows the existing Maud + HTMX dashboard pattern (issue #50).

use crate::auth::CurrentUser;
use crate::components;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::Form;
use memayu_api::WebServices;

#[derive(serde::Deserialize)]
pub struct ChangePasswordForm {
    pub current_password: String,
    pub new_password: String,
    pub confirm: String,
}

#[derive(serde::Deserialize)]
pub struct ChangeEmailForm {
    pub email: String,
}

/// Render the account page body (both forms) with an optional status/error banner.
fn account_content(email: &str, message: Option<&str>, message_is_error: bool) -> maud::Markup {
    let banner = match message {
        Some(m) if message_is_error => {
            maud::html! {
                div class="alert alert-error mb-6" role="alert" x-data="{ open: true }" x-show="open" {
                    svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" {}
                    }
                    span class="flex-1" { (m) }
                    button type="button" class="alert-close" x-on:click="open = false" aria-label="Close alert" {
                        svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                        }
                    }
                }
            }
        }
        Some(m) => {
            maud::html! {
                div class="alert alert-success mb-6" role="alert" x-data="{ open: true }" x-show="open" {
                    svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-5 w-5" fill="none" viewBox="0 0 24 24" {
                        path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" {}
                    }
                    span class="flex-1" { (m) }
                    button type="button" class="alert-close" x-on:click="open = false" aria-label="Close alert" {
                        svg class="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" {
                            path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" {}
                        }
                    }
                }
            }
        }
        None => maud::html! {},
    };

    maud::html! {
        div class="mb-6" {
            h2 class="text-xl font-bold" { "Accounts" }
            p class="text-xs text-base-content/60 mt-1" {
                "Manage the administrator credentials for this Memayu instance."
            }
        }
        (banner)

        div class="grid grid-cols-1 lg:grid-cols-2 gap-6" {
            // ── Change email ──
            div class="card bg-base-100 shadow-sm" {
                div class="card-body" {
                    h3 class="card-title text-lg mb-1" { "Email Address" }
                    p class="text-xs text-base-content/60 mb-4" {
                        "Current email: " strong { (email) }
                    }
                    form method="post" action="/accounts/email" class="space-y-4" {
                        fieldset class="fieldset" {
                            label class="label" { span { "New Email Address" } }
                            input type="email" name="email" class="input w-full"
                                value=(email) required;
                            p class="text-xs text-base-content/50 mt-1" {
                                "Used to log in to the administrator dashboard."
                            }
                        }
                        div class="pt-2" {
                            button type="submit" class="btn btn-primary" { "Update email" }
                        }
                    }
                }
            }

            // ── Change password ──
            div class="card bg-base-100 shadow-sm" {
                div class="card-body" {
                    h3 class="card-title text-lg mb-1" { "Change Password" }
                    p class="text-xs text-base-content/60 mb-4" {
                        "Enter your current password to set a new password."
                    }
                    form method="post" action="/accounts/password" class="space-y-4" {
                        fieldset class="fieldset" {
                            label class="label" { span { "Current Password" } }
                            input type="password" name="current_password" class="input w-full"
                                required autocomplete="current-password";
                        }
                        fieldset class="fieldset" {
                            label class="label" { span { "New Password" } }
                            input type="password" name="new_password" class="input w-full"
                                required autocomplete="new-password"
                                placeholder="Min 8 chars, upper+lower+digit";
                        }
                        fieldset class="fieldset" {
                            label class="label" { span { "Confirm New Password" } }
                            input type="password" name="confirm" class="input w-full"
                                required autocomplete="new-password";
                        }
                        div class="pt-2" {
                            button type="submit" class="btn btn-primary" { "Update password" }
                        }
                    }
                }
            }
        }
    }
}

/// GET /account
pub async fn get_account(user: CurrentUser) -> Result<Html<String>, (StatusCode, String)> {
    Ok(Html(components::render_page(
        "account",
        Some(&user.email),
        "Accounts",
        "Accounts",
        account_content(&user.email, None, false),
    )))
}

/// POST /accounts/email
pub async fn post_account_email(
    user: CurrentUser,
    State(services): State<WebServices>,
    Form(form): Form<ChangeEmailForm>,
) -> Result<Html<String>, (StatusCode, String)> {
    match services.auth_change_email(&user.id, &form.email).await {
        Ok(new_email) => {
            // The session's email is derived from the DB on each resolve, so it
            // refreshes automatically. Re-render with the new email.
            Ok(Html(components::render_page(
                "account",
                Some(&new_email),
                "Accounts",
                "Accounts",
                account_content(&new_email, Some("Email updated."), false),
            )))
        }
        Err(e) => {
            let msg = e.message;
            Ok(Html(components::render_page(
                "account",
                Some(&user.email),
                "Accounts",
                "Accounts",
                account_content(&user.email, Some(&msg), true),
            )))
        }
    }
}

/// POST /accounts/password
pub async fn post_account_password(
    user: CurrentUser,
    State(services): State<WebServices>,
    Form(form): Form<ChangePasswordForm>,
) -> Result<Html<String>, (StatusCode, String)> {
    match services
        .auth_change_password(
            &user.id,
            &form.current_password,
            &form.new_password,
            &form.confirm,
        )
        .await
    {
        Ok(()) => Ok(Html(components::render_page(
            "account",
            Some(&user.email),
            "Accounts",
            "Accounts",
            account_content(&user.email, Some("Password updated."), false),
        ))),
        Err(e) => {
            let msg = e.message;
            Ok(Html(components::render_page(
                "account",
                Some(&user.email),
                "Accounts",
                "Accounts",
                account_content(&user.email, Some(&msg), true),
            )))
        }
    }
}
