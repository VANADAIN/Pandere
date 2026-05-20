use pandere_core::{ChatId, ChatSummary};

use super::{AppState, MessengerFocus, MessengerView, ThreadStatus};

impl AppState {
    pub fn root_chats(&self) -> Vec<&ChatSummary> {
        self.source
            .chats
            .iter()
            .filter(|chat| !is_topic_chat_id(&chat.id))
            .collect()
    }

    pub fn thread_chats(&self) -> Vec<&ChatSummary> {
        let Some(root_chat_id) = self.selected_root_chat_id.as_ref() else {
            return Vec::new();
        };
        self.forum_threads
            .get(root_chat_id)
            .map(|threads| threads.iter().collect())
            .unwrap_or_default()
    }

    pub fn selected_root_chat_index(&self) -> Option<usize> {
        let selected = self.selected_root_chat_id.as_ref()?;
        self.root_chats().iter().position(|chat| &chat.id == selected)
    }

    pub fn selected_thread_chat_index(&self) -> Option<usize> {
        let selected = self.selected_thread_chat_id.as_ref()?;
        self.thread_chats()
            .iter()
            .position(|chat| &chat.id == selected)
    }

    pub fn selected_root_has_threads(&self) -> bool {
        self.selected_root_chat()
            .map(|chat| chat.has_subchats)
            .unwrap_or(false)
    }

    pub fn active_chat_id(&self) -> Option<ChatId> {
        match &self.messenger_view {
            MessengerView::Root => {
                if self.selected_root_has_threads() {
                    None
                } else {
                    self.selected_root_chat_id.clone()
                }
            }
            MessengerView::GroupThreads { .. } => self.selected_thread_chat_id.clone(),
        }
    }

    pub fn preview_chat_id(&self) -> Option<ChatId> {
        match &self.messenger_view {
            MessengerView::Root => {
                if self.selected_root_has_threads() {
                    self.selected_thread_chat_id.clone()
                } else {
                    self.selected_root_chat_id.clone()
                }
            }
            MessengerView::GroupThreads { .. } => self.selected_thread_chat_id.clone(),
        }
    }

    pub fn reset_messenger_selection(&mut self) {
        let first_root = self.root_chats().first().map(|chat| chat.id.clone());
        if self
            .selected_root_chat_id
            .as_ref()
            .is_none_or(|selected| !self.root_chats().iter().any(|chat| chat.id == *selected))
        {
            self.selected_root_chat_id = first_root;
        }
        self.messenger_view = MessengerView::Root;
        self.messenger_focus = MessengerFocus::Left;
        self.sync_selected_thread_to_root();
        self.source.messages.clear();
        self.forum_threads_status = ThreadStatus::Idle;
        self.thread_status = ThreadStatus::Idle;
        self.reset_thread_scroll();
    }

    pub fn set_chats(&mut self, chats: Vec<ChatSummary>) {
        let previously_selected = self.selected_root_chat_id.clone();
        self.source.chats = chats;
        let root_chats = self.root_chats();
        self.selected_root_chat_id = previously_selected
            .filter(|selected| root_chats.iter().any(|chat| chat.id == *selected))
            .or_else(|| root_chats.first().map(|chat| chat.id.clone()));
        self.sync_selected_thread_to_root();
    }

    pub fn select_next_root_chat(&mut self) {
        let root_chats = self.root_chats();
        if root_chats.is_empty() {
            self.selected_root_chat_id = None;
            return;
        }

        let current = self.selected_root_chat_index().unwrap_or(0);
        let next = (current + 1).min(root_chats.len() - 1);
        self.selected_root_chat_id = Some(root_chats[next].id.clone());
        self.sync_selected_thread_to_root();
    }

    pub fn select_previous_root_chat(&mut self) {
        let root_chats = self.root_chats();
        if root_chats.is_empty() {
            self.selected_root_chat_id = None;
            return;
        }

        let current = self.selected_root_chat_index().unwrap_or(0);
        let previous = current.saturating_sub(1);
        self.selected_root_chat_id = Some(root_chats[previous].id.clone());
        self.sync_selected_thread_to_root();
    }

    pub fn select_next_thread_chat(&mut self) {
        let thread_chats = self.thread_chats();
        if thread_chats.is_empty() {
            self.selected_thread_chat_id = None;
            return;
        }

        let current = self.selected_thread_chat_index().unwrap_or(0);
        let next = (current + 1).min(thread_chats.len() - 1);
        self.selected_thread_chat_id = Some(thread_chats[next].id.clone());
    }

    pub fn select_previous_thread_chat(&mut self) {
        let thread_chats = self.thread_chats();
        if thread_chats.is_empty() {
            self.selected_thread_chat_id = None;
            return;
        }

        let current = self.selected_thread_chat_index().unwrap_or(0);
        let previous = current.saturating_sub(1);
        self.selected_thread_chat_id = Some(thread_chats[previous].id.clone());
    }

    pub fn enter_selected_root(&mut self) {
        let Some(root_chat_id) = self.selected_root_chat_id.clone() else {
            return;
        };
        if self.selected_root_has_threads() {
            self.messenger_view = MessengerView::GroupThreads { root_chat_id };
            self.messenger_focus = MessengerFocus::Left;
        }
    }

    pub fn leave_group_threads(&mut self) {
        self.messenger_view = MessengerView::Root;
        self.messenger_focus = MessengerFocus::Left;
    }

    pub fn is_inside_group_threads(&self) -> bool {
        matches!(self.messenger_view, MessengerView::GroupThreads { .. })
    }

    pub fn focus_right_pane(&mut self) {
        self.messenger_focus = MessengerFocus::Right;
        self.follow_thread_bottom = true;
    }

    pub fn focus_left_pane(&mut self) {
        self.messenger_focus = MessengerFocus::Left;
    }

    pub fn can_focus_right_pane(&self) -> bool {
        !self.selected_root_has_threads() || self.is_inside_group_threads()
    }

    pub fn thread_status_label(&self) -> String {
        self.thread_status.label()
    }

    pub fn thread_column_title(&self) -> String {
        if self.is_inside_group_threads() {
            self.selected_root_chat()
                .map(|root_chat| format!("Threads in {}", root_chat.title))
                .unwrap_or_else(|| "Threads".into())
        } else {
            "Preview".into()
        }
    }

    pub fn thread_placeholder(&self) -> String {
        match &self.messenger_view {
            MessengerView::Root => match self.selected_root_chat() {
                Some(_chat) if self.selected_root_has_threads() => match &self.forum_threads_status {
                    ThreadStatus::Idle => {
                        "Supergroup preview. Press Right to enter threads.".into()
                    }
                    ThreadStatus::Loading => "Loading threads...".into(),
                    ThreadStatus::Failed(message) => format!("Failed to load threads: {message}"),
                },
                Some(chat) => format!("Opening {}", chat.title),
                None => "No chat selected".into(),
            },
            MessengerView::GroupThreads { .. } => "Select a thread with Up/Down".into(),
        }
    }

    pub fn scroll_thread_up(&mut self, lines: u16) {
        self.follow_thread_bottom = false;
        self.thread_scroll = self.thread_scroll.saturating_add(lines);
    }

    pub fn scroll_thread_down(&mut self, lines: u16) {
        if self.thread_scroll <= lines {
            self.thread_scroll = 0;
            self.follow_thread_bottom = true;
        } else {
            self.thread_scroll -= lines;
            self.follow_thread_bottom = false;
        }
    }

    pub fn effective_thread_scroll(&self, auto_bottom: u16) -> u16 {
        if self.follow_thread_bottom {
            auto_bottom
        } else {
            auto_bottom.saturating_sub(self.thread_scroll.min(auto_bottom))
        }
    }

    pub(super) fn reset_thread_scroll(&mut self) {
        self.thread_scroll = 0;
        self.follow_thread_bottom = true;
    }

    pub(super) fn sync_selected_thread_to_root(&mut self) {
        let thread_chats = self.thread_chats();
        if thread_chats.is_empty() {
            self.selected_thread_chat_id = None;
            return;
        }

        if self
            .selected_thread_chat_id
            .as_ref()
            .is_some_and(|selected| thread_chats.iter().any(|chat| chat.id == *selected))
        {
            return;
        }

        self.selected_thread_chat_id = thread_chats.first().map(|chat| chat.id.clone());
    }
}

fn is_topic_chat_id(chat_id: &ChatId) -> bool {
    chat_id.as_str().contains(":topic:")
}
