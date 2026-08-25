#![cfg(feature = "design-gallery")]
//! Compile-time-only primitive gallery; never part of a production bundle.

use leptos::prelude::*;

use crate::i18n::{t, t_string, use_i18n};
use crate::icons::Icon;
use crate::primitives::{
    Avatar, AvatarSize, Bubble, BubbleKind, Button, ButtonPreviewState, ButtonSize, ButtonVariant,
    Dialog, DialogBody, DialogClose, DialogContent, DialogFooter, DialogTrigger, Field, IconSize,
    IconView, Input, InputGroup, InputGroupAffix, InputGroupAffixPosition, InputPreviewState,
    InputType, Item, ItemAction, ItemActions, ItemDescription, ItemMedia, ItemTitle, Kbd, KbdKey,
    KbdModifier, Menu, MenuContent, MenuItem, MenuSeparator, MenuSub, MenuSubTrigger, MenuTrigger,
    Message, MessageAlign, MessageAvatar, MessageContent, MessageFooter, MessageGroup,
    MessageHeader, Separator, SeparatorOrientation, Sheet, SheetSide, Skeleton, SkeletonShape,
    Switch, Textarea, TextareaPreviewState, Toast, ToastPreviewState, Tooltip, TooltipTrigger,
    TooltipTriggerAction,
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
    let toast_preview_visible = RwSignal::new(true);
    let toast_auto_visible = RwSignal::new(true);
    let toast_dismiss_count = RwSignal::new(0_u32);
    let tooltip_activation_count = RwSignal::new(0_u32);
    let dialog_open = RwSignal::new(false);
    let dialog_close_count = RwSignal::new(0_u32);
    let sheet_open = RwSignal::new(false);
    let sheet_close_count = RwSignal::new(0_u32);
    let menu_open = RwSignal::new(false);
    let menu_select_count = RwSignal::new(0_u32);
    let menu_close_count = RwSignal::new(0_u32);

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
                                <Kbd modifier=KbdModifier::Primary key=KbdKey::Character('K') />
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

                <section class="ob-design-section" aria-labelledby="design-messages-title">
                    <h2 id="design-messages-title">{move || t!(i18n, design_gallery.messages)}</h2>
                    <div class="ob-design-stack" id="design-messages">
                        <MessageGroup>
                            <Message
                                align=MessageAlign::Start
                                aria_label=move || t_string!(i18n, design_gallery.avatar_ada).to_owned()
                            >
                                <MessageAvatar>
                                    <Avatar
                                        principal_id="principal-ada"
                                        name=move || t_string!(i18n, design_gallery.avatar_ada).to_owned()
                                        size=AvatarSize::Medium
                                    />
                                </MessageAvatar>
                                <MessageContent>
                                    <MessageHeader>{move || t!(i18n, design_gallery.avatar_ada)}</MessageHeader>
                                    <Bubble kind=BubbleKind::Assistant preview_hover=true>
                                        {move || t!(i18n, design_gallery.assistant_message)}
                                    </Bubble>
                                    <MessageFooter><Kbd key=KbdKey::Enter /></MessageFooter>
                                </MessageContent>
                            </Message>
                            <Message
                                align=MessageAlign::End
                                aria_label=move || t_string!(i18n, design_gallery.avatar_zhang).to_owned()
                            >
                                <MessageAvatar>
                                    <Avatar
                                        principal_id="principal-zhang"
                                        name=move || t_string!(i18n, design_gallery.avatar_zhang).to_owned()
                                        size=AvatarSize::Small
                                    />
                                </MessageAvatar>
                                <MessageContent>
                                    <MessageHeader>{move || t!(i18n, design_gallery.avatar_zhang)}</MessageHeader>
                                    <Bubble kind=BubbleKind::User>
                                        {move || t!(i18n, design_gallery.user_message)}
                                    </Bubble>
                                    <MessageFooter>
                                        <Kbd modifier=KbdModifier::Shift key=KbdKey::Enter />
                                    </MessageFooter>
                                </MessageContent>
                            </Message>
                        </MessageGroup>
                        <div class="ob-design-row" id="design-avatar-repeat">
                            <Avatar
                                principal_id="principal-ada"
                                name=move || t_string!(i18n, design_gallery.avatar_ada).to_owned()
                                size=AvatarSize::Large
                            />
                            <Kbd key=KbdKey::Escape />
                            <Kbd key=KbdKey::Slash />
                        </div>
                    </div>
                </section>

                <section class="ob-design-section" aria-labelledby="design-feedback-primitives-title">
                    <h2 id="design-feedback-primitives-title">
                        {move || t!(i18n, design_gallery.feedback_primitives)}
                    </h2>
                    <div class="ob-design-stack" id="design-feedback-primitives">
                        <Toast
                            id="design-toast-preview"
                            visible=toast_preview_visible
                            message=move || t_string!(i18n, design_gallery.toast_preview).to_owned()
                            preview_state=ToastPreviewState::Open
                        />
                        <Toast
                            id="design-toast-auto"
                            visible=toast_auto_visible
                            message=move || t_string!(i18n, design_gallery.toast_auto).to_owned()
                            on_dismiss=UnsyncCallback::new(move |_| toast_dismiss_count.update(|count| *count += 1))
                        />
                        <Button
                            variant=ButtonVariant::Chip
                            on_activate=move |_| toast_auto_visible.set(true)
                        >
                            {move || t!(i18n, design_gallery.show_toast)}
                        </Button>
                        <output id="design-toast-dismiss-count" aria-live="polite">
                            {toast_dismiss_count}
                        </output>
                        <div class="ob-design-row">
                            <Tooltip
                                id="design-tooltip-preview"
                                content=move || t_string!(i18n, design_gallery.tooltip_preview).to_owned()
                                preview_open=true
                            >
                                <TooltipTrigger
                                    id="design-tooltip-preview-trigger"
                                    action=TooltipTriggerAction::Button(UnsyncCallback::new(move |_| {}))
                                >
                                    {move || t!(i18n, design_gallery.tooltip_preview)}
                                </TooltipTrigger>
                            </Tooltip>
                            <Tooltip
                                id="design-tooltip-live"
                                content=move || t_string!(i18n, design_gallery.tooltip_content).to_owned()
                            >
                                <TooltipTrigger
                                    id="design-tooltip-live-trigger"
                                    action=TooltipTriggerAction::Button(UnsyncCallback::new(move |_| tooltip_activation_count.update(|count| *count += 1)))
                                >
                                    {move || t!(i18n, design_gallery.tooltip_trigger)}
                                </TooltipTrigger>
                            </Tooltip>
                        </div>
                        <output id="design-tooltip-count" aria-live="polite">
                            {tooltip_activation_count}
                        </output>
                    </div>
                </section>

                <section class="ob-design-section" aria-labelledby="design-modals-title">
                    <h2 id="design-modals-title">{move || t!(i18n, design_gallery.modals)}</h2>
                    <div class="ob-design-row" id="design-modals">
                        <Dialog
                            id="design-dialog"
                            open=dialog_open
                            on_close=UnsyncCallback::new(move |_| dialog_close_count.update(|count| *count += 1))
                        >
                            <DialogTrigger id="design-dialog-trigger">
                                {move || t!(i18n, design_gallery.dialog_trigger)}
                            </DialogTrigger>
                            <DialogContent
                                title=move || t_string!(i18n, design_gallery.dialog_title).to_owned()
                                description=move || t_string!(i18n, design_gallery.dialog_description).to_owned()
                            >
                                <DialogBody>
                                    <p>{move || t!(i18n, design_gallery.dialog_body)}</p>
                                </DialogBody>
                                <DialogFooter>
                                    <DialogClose id="design-dialog-cancel">
                                        {move || t!(i18n, common.cancel)}
                                    </DialogClose>
                                    <DialogClose id="design-dialog-save">
                                        {move || t!(i18n, common.save)}
                                    </DialogClose>
                                </DialogFooter>
                            </DialogContent>
                        </Dialog>
                        <output id="design-dialog-close-count" aria-live="polite">
                            {dialog_close_count}
                        </output>

                        <Sheet
                            id="design-sheet"
                            open=sheet_open
                            side=SheetSide::Right
                            on_close=UnsyncCallback::new(move |_| sheet_close_count.update(|count| *count += 1))
                        >
                            <DialogTrigger id="design-sheet-trigger">
                                {move || t!(i18n, design_gallery.sheet_trigger)}
                            </DialogTrigger>
                            <DialogContent
                                title=move || t_string!(i18n, design_gallery.sheet_title).to_owned()
                                description=move || t_string!(i18n, design_gallery.sheet_description).to_owned()
                            >
                                <DialogBody>
                                    <p>{move || t!(i18n, design_gallery.dialog_body)}</p>
                                </DialogBody>
                                <DialogFooter>
                                    <DialogClose id="design-sheet-done">
                                        {move || t!(i18n, design_gallery.done)}
                                    </DialogClose>
                                </DialogFooter>
                            </DialogContent>
                        </Sheet>
                        <output id="design-sheet-close-count" aria-live="polite">
                            {sheet_close_count}
                        </output>
                    </div>
                </section>

                <section class="ob-design-section" aria-labelledby="design-menu-title">
                    <h2 id="design-menu-title">{move || t!(i18n, design_gallery.menu)}</h2>
                    <div class="ob-design-row" id="design-menu-example">
                        <Menu
                            id="design-menu"
                            open=menu_open
                            on_close=UnsyncCallback::new(move |_| menu_close_count.update(|count| *count += 1))
                        >
                            <MenuTrigger>{move || t!(i18n, design_gallery.menu_trigger)}</MenuTrigger>
                            <MenuContent>
                                <MenuItem
                                    id="design-menu-new"
                                    on_select=move |_| menu_select_count.update(|count| *count += 1)
                                >
                                    {move || t!(i18n, design_gallery.menu_new)}
                                </MenuItem>
                                <MenuItem
                                    id="design-menu-disabled"
                                    disabled=true
                                    on_select=move |_| menu_select_count.update(|count| *count += 100)
                                >
                                    {move || t!(i18n, design_gallery.menu_disabled)}
                                </MenuItem>
                                <MenuSeparator />
                                <MenuSub id="design-menu-more">
                                    <MenuSubTrigger>
                                        {move || t!(i18n, design_gallery.menu_more)}
                                    </MenuSubTrigger>
                                    <MenuContent>
                                        <MenuItem
                                            id="design-menu-copy"
                                            on_select=move |_| menu_select_count.update(|count| *count += 1)
                                        >
                                            {move || t!(i18n, design_gallery.menu_copy)}
                                        </MenuItem>
                                        <MenuItem
                                            id="design-menu-delete"
                                            on_select=move |_| menu_select_count.update(|count| *count += 1)
                                        >
                                            {move || t!(i18n, design_gallery.menu_delete)}
                                        </MenuItem>
                                    </MenuContent>
                                </MenuSub>
                                <MenuItem
                                    id="design-menu-settings"
                                    on_select=move |_| menu_select_count.update(|count| *count += 1)
                                >
                                    {move || t!(i18n, design_gallery.menu_settings)}
                                </MenuItem>
                            </MenuContent>
                        </Menu>
                        <button id="design-menu-after" type="button" class="ob-button" data-variant="chip" data-size="md">
                            {move || t!(i18n, design_gallery.menu_after)}
                        </button>
                        <output id="design-menu-select-count" aria-live="polite">{menu_select_count}</output>
                        <output id="design-menu-close-count" aria-live="polite">{menu_close_count}</output>
                    </div>
                </section>
            </div>
        </section>
    }
}
