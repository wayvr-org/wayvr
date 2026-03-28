# Quick jump

## [Widgets](#widgets)

[div](#div-widget), [label](#label-widget), [rectangle](#rectangle-widget), [sprite](#sprite-widget)

## [Built-in components](#components)

[Button](#button-component), [Slider](#slider-component), [CheckBox](#checkbox-component), [Tabs](#tabs-component) ([Tab](#tab-component)), [EditBox](#editbox-component), [BarGraph](#bargraph-component)

## [Examples](#examples)

[Simple layout](#simple-layout)

[Value substitution (themes)](#value-substitution-themes)

[File inclusion](#file-inclusion)

[Macros](#macros)

[Templates](#templates)

# Universal widget attributes

_They can be used in any widget/component._

`display`: "flex" | "block" | "grid"

`position`: "absolute" | "relative"

`flex_grow`: **float**

`flex_shrink`: **float**

`gap`: **float** | **percent**

`flex_basis`: **float** | **percent**

`justify_self`: "center" | "end" | "flex_end" | "flex_start" | "start" | "stretch"

`justify_content`: "center" | "end" | "flex_start" | "flex_end" | "space_around" | "space_between" | "space_evenly" | "start" | "stretch"

`flex_wrap`: "wrap" | "no_wrap" | "wrap_reverse"

`flex_direction`: "row" | "column" | "column_reverse" | "row_reverse"

`align_items`, `align_self`: "baseline" | "center" | "end" | "flex_start" | "flex_end" | "start" | "stretch"

`box_sizing`: "border_box" | "content_box"

`margin`, `margin_left`, `margin_right`, `margin_top`, `margin_bottom`: **float** | **percent**

`padding`, `padding_left`, `padding_right`, `padding_top`, `padding_bottom`: **float** | **percent**

`overflow`, `overflow_x`, `overflow_y`: "hidden" | "visible" | "clip" | "scroll"

`min_width`, `min_height`: **float** | **percent**

`max_width`, `max_height`: **float** | **percent**

`width`, `height`: **float** | **percent**

## Advanced attributes

`interactable`: "1" | "0"

_Set to 0 if you want to exclude this widget from altering the event state_

`consume_mouse_events`: "1" | "0"

_Used in case of overlapping pop-ups or windows, most notably applied to various backgrounds_

`new_pass`: "1" | "0"

_Set to 1 if you want to render overlapping pop-ups to properly render your widgets in order. Wgui renders with as few Vulkan drawcalls as possible, so this is your responsibility._

## Default Colors

_These colors can be defined by the user to control the color scheme. They always exists and can be used in any place a normal color could be used._

`~color_text`: default: white(#FFFFFF)
`~color_accent`: default: light blue(#21ADFF)
`~color_danger`: default: red(#E60000)
`~color_faded`: default: grey(#ABBDCC)
`~color_bg`: default: black(#00121ABF) (color_background in config)

_Only the default colors can be changed with the following suffixes._

`_translucent`

_Halves the alpha of the color._

`_10`
`_20`
`_30`
`_40`
`_50`

_Darkens the color by multiplying it with a percentage. `~color_accent_50` Would be half of the normal brightness but the same alpha. You can't combine the suffixes_

### Color

# Widgets

## div widget

### `<div>`

### The most simple element

#### Parameters

_None_

---

## label widget

### `<label>`

### A simple text element

#### Parameters

`text`: **string**

_Simple text_

`translation`: **string**

_Translated by key_

`size`: **float** (default: 14)

_Text size in pixel units_

`wrap`: "1" | "0" (default: "0")

_Enable text wrapping?_

`color`: #FFAABB | #FFAABBCC

`align`: "left" | "right" | "center" | "justified" | "end"

`weight`: "normal" | "bold"

`shadow`: #112233 | #112233CC (default: None)

`shadow_x`: **float** (default: 1.5)

_Horizontal offset of the shadow from the original text. Positive is right._

`shadow_y`: **float** (default: 1.5)

_Vertical offset of the shadow from the original text. Positive is down._

---

## rectangle widget

### `<rectangle>`

### A styled rectangle

#### Parameters

`color`: #FFAABB | #FFAABBCC

_1st gradient color_

`color2`: #FFAABB | #FFAABBCC

_2nd gradient color_

`gradient`: "horizontal" | "vertical" | "radial" | "none"

`round`: **float** (default: 0) | **percent** (0-100%)

`border`: **float**

`border_color`: #FFAABB | #FFAABBCC

---

## sprite widget

### `<sprite>`

### Image widget for small images

Always set the `width` & `height` of a `<sprite>`!

Supported formats: svg, png, jpeg, gif, webp

Maximum image size: 256x256 pixels

For large or frequently changing images (e.g. album art), or if borders/rounding is desired, consider the `<image>` tag instead.

Sprite images are atlassed, making them very efficient to render.

Adding large sprites permanently increases the atlas size (and thus VRAM usage) for the session. (Atlas space can be re-used, but the atlas won't shrink back.)

#### Parameters

`src`: **string**

_External (filesystem) image path. Falls back to Internal (assets) if not found._

`src_ext`: **string**

_External (filesystem) image path_

`src_builtin`: **string**

_Internal (assets) image path_

`src_internal`: **string**

_wgui internal image path. Do not use directly unless it's related to the core wgui assets._

## image widget

### `<image>`

### Image widget for large images

Always set the `width` & `height` of an `<image>`!

Supported formats: svg, png, jpeg, gif, webp

Maximum image size: Max texture size for the GPU (usually 8K+)

For small images such as icons, consider using the `<sprite>` tag instead.

`<image>` requires a single draw call per widget, while `<sprite>` widgets all share a single draw call per panel.

#### Parameters

`src`: **string**

_External (filesystem) image path. Falls back to Internal (assets) if not found._

`src_ext`: **string**

_External (filesystem) image path_

`src_builtin`: **string**

_Internal (assets) image path_

`src_internal`: **string**

_wgui internal image path. Do not use directly unless it's related to the core wgui assets._

`round`: **float** (default: 0) | **percent** (0-100%)

`border`: **float**

`border_color`: #FFAABB | #FFAABBCC

---

# Components

## Button component

### `<Button>`

### A clickable, decorated button

#### Parameters

`text`: **string**

_Simple text_

`translation`: **string**

_Translated by key_

`round`: **float** (default: 4) | **percent** (0-100%)

`border`: **float** (default: 2)

`color`: #FFAABB | #FFAABBCC

`border_color`: #FFAABB | #FFAABBCC

`hover_color`: #FFAABB | #FFAABBCC

`hover_border_color`: #FFAABB | #FFAABBCC

`tooltip`: **string**

_Tooltip text on hover, translated by key_

`tooltip_str`: **string**

_Tooltip text on hover, raw text (not translated)_

`tooltip_side`: "top" | "bottom" | "left" | "right" (default: top)

`sticky`: "1" | "0" (default: "0")

_make button act as a toggle (visual only)_

`sprite_src` | `sprite_src_ext` | `sprite_src_builtin` | `sprite_src_internal`

_Image path (see [sprite](#sprite-widget)) for src descriptions_

#### Info

Child widgets are supported and can be added directly in XML.

---

## Slider component

### `<Slider>`

### A simple slider.

#### Parameters

`min_value`: **float**

`max_value`: **float**

_Needs to be bigger than `min_value`_

`value`: **float**

_Initial slider value_

`show_value`: "1" | "0"

_Set to 0 if you don't want to display value text in the slider handle_

---

## Checkbox component

### `<CheckBox>`

### A check-box with label.

#### Parameters

`text`: **string**

_Simple text_

`translation`: **string**

_Translated by key_

`box_size`: **float** (default: 24)

`color_checked`: #FFAABB | #FFAABBCC

`value`: **string**

_optional value that will be sent with internal events_

`checked`: **int** (default: 0)

---

## Radiobox component

### `<RadioBox>`

### A radio-box with label.

#### Parameters

`text`: **string**

_Simple text_

`translation`: **string**

_Translated by key_

`box_size`: **float** (default: 24)

`value`: **string**

_optional value that will be set as the RadioGroup's value_

`checked`: **int** (default: 0)

---

## RadioGroup component

### `<RadioGroup>`

### A radio group. Place `<RadioBox>` components inside this.

## Tabs component

### `<Tabs>`

### A tab bar component

#### Parameters

`id`: **string**

_Unique identifier for the tab group_

Only `<Tab>` components should be placed inside `<Tabs>`.

---

## Tab component

### `<Tab>`

### A single tab inside a `<Tabs>` component

#### Parameters

`name`: **string**

_Unique identifier for the tab_

`translation`: **string**

_Translated by key_

`sprite_src` | `sprite_src_ext` | `sprite_src_builtin` | `sprite_src_internal`

_Image path (see [sprite](#sprite-widget)) for src descriptions_

---

## EditBox component

### `<EditBox>`

### A single-line text input field.

#### Parameters

`text`: **string**

_Initial text content_

## BarGraph component

### `<BarGraph>`

### A bar graph widget

#### Parameters

`limit_min`: **float**

_Minimum limit value_

`limit_max`: **float**

_Maximum limit value_

`unit`: **string** (default: "")

_Unit type, for example "%" or "ms"_

`capacity`: **int** (default: 50)

_Value count_

---

# Examples

## Simple layout

```xml
<layout>
  <elements>
    <label text="Hello, world!"/>
    <label translation="WELCOME.HELLO_WORLD" size="20" color="#FF0000"/>
    <div gap="16" flex_direction="row">
      <rectangle width="16" height="16" color="#FF0000"/>
      <rectangle width="16" height="16" color="#00FF00"/>
      <rectangle width="16" height="16" color="#0000FF"/>
    </div>
  </elements>
</layout>
```

## Value substitution (themes)

```xml
<layout>
  <theme>
    <var key="hello" value="Hello, world!" />
    <var key="text_color" value="#FF0000" />
  </theme>

  <elements>
    <!-- "~hello" will be replaced to "Hello, world!" -->
    <label text="~hello"/>
    <!-- "~text_color" will be replaced to "#FF0000" -->
    <label text="This text will be red" color="~text_color"/>
  </elements>
</layout>
```

## Macros

```xml
<layout>
  <macro name="my_macro"
    margin="4" min_width="100" min_height="100" flex_direction="row" gap="8"
    align_items="center" justify_content="center"/>

  <elements>
    <!-- This div will have all attributes specified in "my_macro" -->
    <div macro="my_macro">
      <label text="hello, world!"/>
    </div>
  </elements>
</layout>

```

## File inclusion

theme.xml:

```xml
<layout>
  <theme>
    <var key="my_red" value="#FF0000" />
    <var key="my_green" value="#00FF00" />
    <var key="my_blue" value="#0000FF" />
  </theme>
</layout>
```

bar.xml:

```xml
<layout>
  <elements>
    <!-- utilize theme variables included in theme.xml -->
    <rectangle width="16" height="16" color="~my_red"/>
    <rectangle width="16" height="16" color="~my_green"/>
    <rectangle width="16" height="16" color="~my_blue"/>
  </elements>
</layout>
```

main.xml:

```xml
<layout>
  <!-- Include theme -->
  <include src="theme.xml"/>

  <elements>
    <!-- Include as additional elements here -->
    <include src="bar.xml"/>
  </elements>
</layout>
```

## Templates

```xml
<layout>
  <!-- "title" attrib will be passed to every matching ${title} -->
  <template name="DecoratedTitle">
    <rectangle color="#FFFF00" padding="8" round="4" gap="4">
      <label text="${title}"/>
    </rectangle>
  </template>

  <elements>
    <!-- "title" used here -->
    <DecoratedTitle title="This is a title.">
  </elements>
</layout>

```
