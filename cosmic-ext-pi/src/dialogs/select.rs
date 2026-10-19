use crate::subscription::PiAction;
use cosmic::widget;
use cosmic::Element;
use pi_rpc::protocol::ExtensionUiResponse;

/// Accessible selection dialog for Extension UI requests.
pub struct SelectDialog;

impl SelectDialog {
    /// Builds a native libcosmic dialog element for option selection.
    pub fn view<'a>(
        id: &'a str,
        title: &'a str,
        options: &'a [String],
        selected: Option<usize>,
    ) -> Element<'a, PiAction> {
        let id_cancel = id.to_string();
        let id_select = id.to_string();
        let id_dropdown = id.to_string();

        let cancel_btn = widget::button::standard("Cancel")
            .description("Cancel selection")
            .on_press(PiAction::DialogCancel(id_cancel));

        let selected_val = selected.and_then(|idx| options.get(idx).cloned());

        let mut select_btn =
            widget::button::suggested("Select").description("Confirm chosen selection");

        if let Some(val) = selected_val {
            select_btn = select_btn.on_press(PiAction::RespondUi(ExtensionUiResponse::value(
                id_select, val,
            )));
        }

        let dropdown_widget = widget::dropdown(options, selected, move |idx| {
            PiAction::DialogSelectOption(id_dropdown.clone(), idx)
        });

        let content = widget::column::with_capacity(2)
            .push(dropdown_widget)
            .spacing(12);

        widget::dialog()
            .title(title)
            .control(content)
            .secondary_action(cancel_btn)
            .primary_action(select_btn)
            .into()
    }
}
