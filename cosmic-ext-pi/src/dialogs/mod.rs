pub mod confirm;
pub mod editor;
pub mod input;
pub mod select;

pub use confirm::ConfirmDialog;
pub use editor::EditorDialog;
pub use input::InputDialog;
pub use select::SelectDialog;

/// Internal state associated with interactive extension dialogs.
#[derive(Debug, Clone)]
pub enum DialogState {
    Confirm,
    Select {
        selected: Option<usize>,
    },
    Input {
        value: String,
        placeholder: Option<String>,
    },
    Editor {
        content: cosmic::iced::widget::text_editor::Content,
    },
}
