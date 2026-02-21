/// Selection changed event — fired by SegmentedControl, TabBar, Sidebar, TableView, ContextMenu.
pub struct SelectionChangedEvent {
    /// The control ID whose selection changed.
    pub id: u32,
    /// The newly selected index.
    pub index: u32,
}
