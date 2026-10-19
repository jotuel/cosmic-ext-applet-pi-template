use crate::subscription::PiAction;
use cosmic::widget;
use cosmic::Element;
use pi_rpc::protocol::ExtensionUiResponse;

/// Accessible multi-line editor dialog for Extension UI requests.
pub struct EditorDialog;

impl EditorDialog {
    /// Builds a native libcosmic dialog element for multi-line text editing.
    pub fn view<'a>(
        id: &'a str,
        title: &'a str,
        content: &'a cosmic::iced::widget::text_editor::Content,
    ) -> Element<'a, PiAction> {
        let id_cancel = id.to_string();
        let id_submit = id.to_string();
        let id_editor = id.to_string();
        let full_text = content.text();

        let cancel_btn = widget::button::standard("Cancel")
            .description("Cancel editing")
            .on_press(PiAction::DialogCancel(id_cancel));

        let submit_btn = widget::button::suggested("Submit")
            .description("Submit edited text")
            .on_press(PiAction::RespondUi(ExtensionUiResponse::value(
                id_submit, full_text,
            )));

        let editor_widget = widget::text_editor::text_editor(content)
            .on_action(move |action| PiAction::EditorAction(id_editor.clone(), action))
            .height(cosmic::iced::Length::Fixed(240.0));

        let container = widget::container(editor_widget)
            .padding(4)
            .width(cosmic::iced::Length::Fill);

        widget::dialog()
            .title(title)
            .control(container)
            .secondary_action(cancel_btn)
            .primary_action(submit_btn)
            .into()
    }
}
