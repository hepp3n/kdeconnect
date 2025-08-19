pub(crate) trait PluginHandler {
    #[must_use]
    async fn pair(&self, flag: bool);
    #[must_use]
    async fn ping(&self, message: String);
}
