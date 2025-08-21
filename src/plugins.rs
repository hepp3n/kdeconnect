pub(crate) trait PluginHandler {
    #[must_use]
    async fn ping(&self, message: String);
}
