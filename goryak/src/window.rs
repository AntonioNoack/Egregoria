use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use yakui_core::geometry::{Color, Constraints, Dim2, Vec2};
use yakui_core::widget::{LayoutContext, Widget};
use yakui_core::{context, Alignment, Flow};
use yakui_widgets::widgets::{Button, Pad};
use yakui_widgets::{center, constrained, draggable, offset, reflow};

use crate::{
    blur_bg, divider, icon_button, mincolumn, on_primary_container, outline, primary_container,
    textc,
};

thread_local! {
    /// Remember which windows were drawn. That what we can put them at the bottom of the widget tree to be drawn on top of the rest.
    /// We can also have a way to remember which windows are active, so they can be on top too.
    static WINDOWS: RefCell<Vec<Window>> = Default::default();
}

pub struct Window {
    pub title: &'static str,
    pub pad: Pad,
    pub radius: f32,
}

impl Window {
    pub fn show(self, children: impl FnOnce(), on_close: impl FnOnce()) {
        let dom = context::dom();
        let response = dom.begin_widget::<WindowBase>(());

        let off = draggable(|| {
            blur_bg(primary_container().with_alpha(0.5), self.radius, || {
                self.pad.show(|| {
                    mincolumn(|| {
                        reflow(Alignment::TOP_RIGHT, Dim2::ZERO, || {
                            offset(Vec2::new(-25.0, -15.0), || {
                                constrained(Constraints::tight(Vec2::splat(40.0)), || {
                                    center(|| {
                                        let mut b = Button::unstyled("close");
                                        b.padding = Pad::balanced(4.0, 2.0);
                                        b.border_radius = 10.0;
                                        b.style.fill = Color::CLEAR;
                                        b.style.text.font_size = 20.0;
                                        b.style.text.color = on_primary_container().adjust(0.5);
                                        b.down_style.fill = Color::CLEAR;
                                        b.down_style.text = b.style.text.clone();
                                        b.hover_style.fill = Color::CLEAR;
                                        b.hover_style.text = b.style.text.clone();
                                        b.hover_style.text.font_size = 25.0;
                                        b.hover_style.text.color = on_primary_container();

                                        if icon_button(b).show().clicked {
                                            on_close();
                                        }
                                    });
                                });
                            });
                        });
                        textc(on_primary_container(), Cow::Borrowed(self.title));
                        divider(outline(), 10.0, 1.0);
                        children();
                    });
                });
            });
        });

        response.confirm.set(off.dragging.is_none());
        if let Some(drag) = off.dragging {
            response.off.set(drag.current - drag.start);
        }

        dom.end_widget::<WindowBase>(response.id);
    }
}

#[derive(Default, Debug)]
struct WindowBase {
    props: (),
    off: Vec2,
    resp: Rc<WindowResp>,
}

#[derive(Default, Debug)]
struct WindowResp {
    off: Cell<Vec2>,
    confirm: Cell<bool>,
}

impl Widget for WindowBase {
    type Props<'a> = ();
    type Response = Rc<WindowResp>;

    fn new() -> Self {
        Self::default()
    }

    fn update(&mut self, props: Self::Props<'_>) -> Self::Response {
        self.props = props;
        if self.resp.confirm.get() {
            self.off += self.resp.off.get();
            self.resp.off.set(Vec2::ZERO);
        }
        self.resp.clone()
    }

    fn flow(&self) -> Flow {
        Flow::Relative {
            anchor: Alignment::TOP_LEFT,
            offset: Dim2::ZERO,
        }
    }

    fn layout(&self, mut ctx: LayoutContext<'_>, _: Constraints) -> Vec2 {
        let node = ctx.dom.get_current();
        if node.children.len() > 1 {
            panic!("Window can only have one child");
        }

        let child = *node.children.first().unwrap();
        let size = ctx.calculate_layout(child, Constraints::loose(ctx.layout.viewport().size()));

        let vp = ctx.layout.viewport().size();

        let mut pos = vp * 0.5 - size * 0.5 + self.off + self.resp.off.get();
        let overflow = (pos + size - vp).max(Vec2::ZERO);
        pos -= overflow;
        pos = pos.max(Vec2::ZERO);

        ctx.layout.set_pos(child, pos);

        Vec2::ZERO
    }
}
