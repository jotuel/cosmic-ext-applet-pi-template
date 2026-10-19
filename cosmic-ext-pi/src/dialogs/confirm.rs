use crate::subscription::PiAction;
use cosmic::widget;
use cosmic::Element;

/// Accessible confirmation dialog for Extension UI requests (e.g. tool execution authorization).
pub struct ConfirmDialog;

impl ConfirmDialog {
    /// Builds a native libcosmic dialog element for confirmation.
    pub fn view<'a>(id: &'a str, title: &'a str, message: &'a str) -> Element<'a, PiAction> {
        let id_allow = id.to_string();
        let id_deny = id.to_string();

        let deny_btn = widget::button::standard("Deny")
            .description("Deny the requested operation")
            .on_press(PiAction::DialogConfirm(id_deny, false));

        let allow_btn = widget::button::suggested("Allow")
            .description("Allow the requested operation")
            .on_press(PiAction::DialogConfirm(id_allow, true));

        widget::dialog()
            .title(title)
            .body(message)
            .secondary_action(deny_btn)
            .primary_action(allow_btn)
            .into()
    }
}
