use sailfish::TemplateOnce;

#[derive(TemplateOnce)]
#[template(path = "activity_main.xml", delimiter = '#')]
pub struct ActivityXml {}

impl ActivityXml {
    pub fn new() -> ActivityXml {
        ActivityXml {}
    }
}

impl Default for ActivityXml {
    fn default() -> Self {
        Self::new()
    }
}
