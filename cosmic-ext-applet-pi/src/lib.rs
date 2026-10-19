// SPDX-License-Identifier: EUPL-1.2

pub mod app;
pub mod config;
pub mod file_reference;
pub mod i18n;
pub mod notifications;
pub mod popover;

pub fn run() -> cosmic::iced::Result {
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();
    i18n::init(&requested_languages);
    cosmic::applet::run::<app::AppModel>(())
}
