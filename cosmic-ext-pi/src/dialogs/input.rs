use crate::subscription::PiAction;
use cosmic::widget;
use cosmic::Element;
use pi_rpc::protocol::ExtensionUiResponse;

/// Accessible single-line input dialog for Extension UI requests.
pub struct InputDialog;

impl InputDialog {
    /// Builds a native libcosmic dialog element for text input.
    pub fn view<'a>(
        id: &'a str,
        title: &'a str,
        placeholder: Option<&'a str>,
        value: &'a str,
    ) -> Element<'a, PiAction> {
        let id_cancel = id.to_string();
        let id_input = id.to_string();
        let id_submit = id.to_string();
        let val_submit = value.to_string();

        let cancel_btn = widget::button::standard("Cancel")
            .description("Cancel input")
            .on_press(PiAction::DialogCancel(id_cancel));

        let submit_btn = widget::button::suggested("Submit")
            .description("Submit input text")
            .on_press(PiAction::RespondUi(ExtensionUiResponse::value(
                id_submit.clone(),
                val_submit,
            )));

        let input_widget = widget::text_input(placeholder.unwrap_or(""), value)
            .on_input(move |text| PiAction::DialogInputChanged(id_input.clone(), text))
            .on_submit(move |text| {
                PiAction::RespondUi(ExtensionUiResponse::value(id_submit.clone(), text))
            });

        widget::dialog()
            .title(title)
            .control(input_widget)
            .secondary_action(cancel_btn)
            .primary_action(submit_btn)
            .into()
    }
}
