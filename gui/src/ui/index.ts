/**
 * The public surface of the primitive layer.
 *
 * Views import from here (`import { Button, Chip } from '../ui'`) and never from a
 * file inside. That is not ceremony: it is what lets a primitive be split, renamed
 * or merged -- `Drawer` moved into `Modal.tsx`, `ActionButton` became `Button` with
 * a `tier` -- without a diff in every consumer, which is how the old tree ended up
 * with three components for one button.
 *
 * The machinery is deliberately absent: `cx`, `place`, `usePopover`,
 * `useOutsideDismiss`, `portal`. A view that reaches for one of those is
 * re-implementing a primitive rather than using one, and the tests that need them
 * import the module path directly.
 */

export { Button, IconButton } from './Button';
export type { ButtonProps } from './Button';
export { Card } from './Card';
export { Callout } from './Callout';
export { Checkbox } from './Checkbox';
export { Chip } from './Chip';
export { confirm, Confirm } from './Confirm';
export type { ConfirmOptions, ConfirmProps } from './Confirm';
export { CopyButton } from './CopyButton';
export { EmptyState } from './EmptyState';
export { Field, useField, useFieldProps } from './Field';
export type { FieldInfo } from './Field';
export { Icon } from './Icon';
export { SearchInput, TextArea, TextInput } from './Input';
export type { TextAreaProps, TextInputProps } from './Input';
export { KeyValue } from './KeyValue';
export { NumberField } from './NumberField';
export type { NumberFieldProps } from './NumberField';
export { Menu } from './Menu';
export type { MenuEntry, MenuItem, MenuSeparator } from './Menu';
export { Drawer, Modal } from './Modal';
export { PopConfirm } from './PopConfirm';
export type { PopConfirmProps } from './PopConfirm';
export { Popover } from './Popover';
export type { PopoverProps } from './Popover';
export { Radio, RadioGroup } from './Radio';
export { Segmented } from './Segmented';
export type { SegmentedOption } from './Segmented';
export { Select } from './Select';
export type { SelectOption } from './Select';
export { Skeleton } from './Skeleton';
export { Slider } from './Slider';
export { Spinner } from './Spinner';
export { Stat } from './Stat';
export { StatusDot } from './StatusDot';
export { Switch } from './Switch';
export { Tabs } from './Tabs';
export type { TabItem } from './Tabs';
export { ToastViewport } from './Toast';
export { clearToasts, dismissToast, pushToast, useToasts } from './toastStore';
export type { ToastItem, ToastTone } from './toastStore';
export { TOOLTIP_DELAY, Tooltip, useTooltip } from './Tooltip';
export type { TooltipController } from './Tooltip';
export { Wordmark } from './Wordmark';
export type { Align, CalloutTone, ChipStatus, Side, Size, Tier } from './types';
