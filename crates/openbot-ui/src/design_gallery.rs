#![cfg(feature = "design-gallery")]
//! Compile-time-only primitive gallery; never part of a production bundle.

use leptos::prelude::*;

use crate::i18n::{t, t_string, use_i18n};
use crate::icons::Icon;
use crate::primitives::{
    Button, ButtonPreviewState, ButtonSize, ButtonVariant, Field, IconSize, IconView, Input,
    InputGroup, InputGroupAffix, InputGroupAffixPosition, InputPreviewState, InputType, Item,
    ItemAction, ItemActions, ItemDescription, ItemMedia, ItemTitle, Separator,
    SeparatorOrientation, Skeleton, SkeletonShape, Switch, Textarea, TextareaPreviewState,
};

/// Render the first Batch 17 primitive states for golden, keyboard and AX inspection.
#[component]
pub fn DesignGallery() -> impl IntoView {
    let i18n = use_i18n();
    let button_count = RwSignal::new(0_u32);
    let item_count = RwSignal::new(0_u32);
    let name = RwSignal::new(String::new());
    let search = RwSignal::new(String::new());
    let notes = RwSignal::new(String::new());
    let enabled = RwSignal::new(false);
    let disabled_checked = RwSignal::new(true);

    view! {
        <section class="ob-page ob-design-gallery" aria-labelledby="design-gallery-title">
            <header class="ob-page-header">
                <div>
                    <p class="ob-eyebrow">{move || t!(i18n, design_gallery.eyebrow)}</p>
                    <h1 id="design-gallery-title" class="ob-page-title">
                        {move || t!(i18n, design_gallery.title)}
                    </h1>
                    <p class="ob-page-intro">{move || t!(i18n, design_gallery.intro)}</p>
                </div>
            </header>

            <div class="ob-design-grid">
                <section class="ob-design-section" aria-labelledby="design-buttons-title">
                    <h2 id="design-buttons-title">{move || t!(i18n, design_gallery.buttons)}</h2>
                    <div class="ob-design-row" id="design-buttons">
                        <Button
                            variant=ButtonVariant::Chip
                            size=ButtonSize::Small
                            on_activate=move |_| button_count.update(|count| *count += 1)
                        >
                            {move || t!(i18n, design_gallery.activate)}
                        </Button>
                        <Button
                            variant=ButtonVariant::Primary
                            size=ButtonSize::Medium
                            selected=true
                            on_activate=move |_| button_count.update(|count| *count += 1)
                        >
                            {move || t!(i18n, common.confirm)}
                        </Button>
                        <Button
                            variant=ButtonVariant::Ghost
                            size=ButtonSize::Large
                            open=true
                            preview_state=ButtonPreviewState::FocusVisible
                            on_activate=move |_| button_count.update(|count| *count += 1)
                        >
                            {move || t!(i18n, common.open)}
                        </Button>
                        <Button
                            variant=ButtonVariant::DangerText
                            size=ButtonSize::Medium
                            invalid=true
                            on_activate=move |_| button_count.update(|count| *count += 1)
                        >
                            {move || t!(i18n, common.delete)}
                        </Button>
                        <Button
                            loading=true
                            preview_state=ButtonPreviewState::Hover
                            on_activate=move |_| button_count.update(|count| *count += 1)
                        >
                            {move || t!(i18n, common.loading)}
                        </Button>
                        <Button
                            disabled=true
                            preview_state=ButtonPreviewState::Active
                            on_activate=move |_| button_count.update(|count| *count += 1)
                        >
                            {move || t!(i18n, common.disabled)}
                        </Button>
                    </div>
                    <output id="design-button-count" aria-live="polite">{button_count}</output>
                </section>

                <section class="ob-design-section" aria-labelledby="design-fields-title">
                    <h2 id="design-fields-title">{move || t!(i18n, design_gallery.fields)}</h2>
                    <div class="ob-design-stack" id="design-fields">
                        <Field
                            control_id="design-name"
                            label=move || t_string!(i18n, design_gallery.name_label).to_owned()
                            description=move || t_string!(i18n, design_gallery.name_description).to_owned()
                        >
                            <Input value=name input_type=InputType::Text />
                        </Field>
                        <Field
                            control_id="design-notes"
                            label=move || t_string!(i18n, design_gallery.notes_label).to_owned()
                            error=move || t_string!(i18n, errors.validation_invalid).to_owned()
                            invalid=true
                        >
                            <Textarea
                                value=notes
                                preview_state=TextareaPreviewState::Focus
                            />
                        </Field>
                        <Field
                            control_id="design-disabled-input"
                            label=move || t_string!(i18n, common.disabled).to_owned()
                            disabled=true
                        >
                            <Input value=name input_type=InputType::Text />
                        </Field>
                        <InputGroup preview_focus_within=true>
                            <InputGroupAffix position=InputGroupAffixPosition::Prefix>
                                <IconView icon=Icon::Search size=IconSize::Inline />
                            </InputGroupAffix>
                            <Input
                                value=search
                                input_type=InputType::Search
                                id="design-search"
                                aria_label=move || t_string!(i18n, shell.search).to_owned()
                                preview_state=InputPreviewState::Focus
                            />
                            <InputGroupAffix position=InputGroupAffixPosition::Suffix>
                                <span>"⌘K"</span>
                            </InputGroupAffix>
                        </InputGroup>
                    </div>
                </section>

                <section class="ob-design-section" aria-labelledby="design-items-title">
                    <h2 id="design-items-title">{move || t!(i18n, design_gallery.items)}</h2>
                    <div class="ob-design-stack" id="design-items">
                        <Item
                            action=ItemAction::Link("/approvals".to_owned())
                            selected=true
                            preview_hover=true
                        >
                            <ItemMedia><IconView icon=Icon::ListChecks size=IconSize::Navigation /></ItemMedia>
                            <ItemTitle>{move || t!(i18n, admin.nav_approvals)}</ItemTitle>
                            <ItemDescription>{move || t!(i18n, design_gallery.item_description)}</ItemDescription>
                        </Item>
                        <Item
                            action=ItemAction::Button(UnsyncCallback::new(move |_| item_count.update(|count| *count += 1)))
                        >
                            <ItemMedia><IconView icon=Icon::Settings size=IconSize::Navigation /></ItemMedia>
                            <ItemTitle>{move || t!(i18n, shell.nav_settings)}</ItemTitle>
                            <ItemActions>
                                <IconView icon=Icon::ChevronRight size=IconSize::Inline />
                            </ItemActions>
                        </Item>
                        <Item
                            action=ItemAction::Button(UnsyncCallback::new(move |_| item_count.update(|count| *count += 1)))
                            disabled=true
                        >
                            <ItemTitle>{move || t!(i18n, common.disabled)}</ItemTitle>
                        </Item>
                        <output id="design-item-count" aria-live="polite">{item_count}</output>
                    </div>
                </section>

                <section class="ob-design-section" aria-labelledby="design-feedback-title">
                    <h2 id="design-feedback-title">{move || t!(i18n, design_gallery.feedback)}</h2>
                    <div class="ob-design-stack" id="design-feedback">
                        <Field
                            control_id="design-switch"
                            label=move || t_string!(i18n, design_gallery.switch_label).to_owned()
                        >
                            <Switch checked=enabled />
                        </Field>
                        <Switch
                            checked=disabled_checked
                            aria_label=move || t_string!(i18n, common.disabled).to_owned()
                            disabled=true
                        />
                        <Separator decorative=true />
                        <div class="ob-design-row">
                            <Skeleton shape=SkeletonShape::Circle />
                            <Skeleton shape=SkeletonShape::Line />
                        </div>
                        <Skeleton shape=SkeletonShape::Block />
                        <Separator orientation=SeparatorOrientation::Horizontal />
                        <Separator orientation=SeparatorOrientation::Vertical />
                    </div>
                </section>
            </div>
        </section>
    }
}
