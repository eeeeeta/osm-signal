use serde::Serialize;
use std::borrow::Cow;
use handlebars::Handlebars;
use super::Result;
use rouille::Response;

#[derive(Serialize)]
pub struct TemplateContext<'a, T> where T: Serialize {
    pub template: &'static str,
    pub title: Cow<'a, str>,
    pub body: T
}
impl<'a, T> TemplateContext<'a, T> where T: Serialize {
    pub fn render(self, hbs: &Handlebars) -> Result<Response> {
        match hbs.render(self.template, &self) {
            Ok(d) => Ok(Response::html(d)),
            Err(e) => {
                error!("Failed to render template: {}", e);
                Err(e.into())
            }
        }
    }
}
impl<'a> TemplateContext<'a, ()> {
    pub fn title<U: Into<Cow<'a, str>>>(template: &'static str, title: U) -> Self {
        TemplateContext {
            template,
            title: title.into(),
            body: ()
        }
    }
}
struct Partial {
    name: &'static str,
    content: &'static str
}
macro_rules! partial {
    ($name:expr) => {
        Partial { 
            name: $name,
            content: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/", $name, ".html.hbs"))
        }
    }
}
static PARTIALS: [Partial; 16] = [
    partial!("connections"),
    partial!("footer"),
    partial!("header"),
    partial!("index"),
    partial!("ise"),
    partial!("map"),
    partial!("movement_search"),
    partial!("movements"),
    partial!("nav"),
    partial!("not_found"),
    partial!("orig_dest"),
    partial!("schedule"),
    partial!("schedule_table"),
    partial!("schedules"),
    partial!("user_error"),
    partial!("train"),
];
pub fn handlebars_init() -> Result<Handlebars> {
    let mut hbs = Handlebars::new();
    hbs.set_strict_mode(true);
    for partial in PARTIALS.iter() {
        hbs.register_partial(partial.name, partial.content)?;
    }
    Ok(hbs)
}
