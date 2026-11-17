---
name: fastdraw-language
description: FastDraw's diagram language, for the jobs that draw.
---

# FastDraw's language

A diagram is FastDraw text (`.fdraw`): one statement per line, every element placed at exact
coordinates. Nobody looks at it before the user does, and you can't render it, so place things
with room to spare and follow the numbers below.

## Rules

- No `look` line and no images or icons from files: the built-in look only.
- At most about 12 boxes and 12 arrows. If two boxes always go together, make them one.
- Labels in the note's own words. Draw only what the note says.
- Positions and sizes in multiples of 4. Start at `40,40`; the picture gets a margin by itself.
- Give every box an id: `api: node "Orders API" at 200,40`. Ids are short, unique, made of
  letters, digits and `_`.
- One focus: `focal`, `terminal-focal` and the `primary` arrow are the accent; at most two of
  them in a diagram.
- End with a `legend` listing only the styles you used.

## Shapes

`id: style "text" at x,y WxH props`

- `at x,y` is the top-left corner, no space after the comma. `WxH` (`160x48`) is the size; leave
  it out for the style's default.
- `"text"` is the main label; `\n` breaks a line. `sub="..."` is a smaller second line (a
  detail, a number, a product). `tag="API"` is a 3–4 letter marker for architecture boxes.
- `fill=`, `stroke=`, `sw=`, `dash=4,3`, `radius=8` override one element's look. Colours are
  `ink`, `muted`, `soft`, `accent`, `link`, `#hex`, or any of them with `/opacity`
  (`ink/0.3`). Prefer the styles; override only with a reason.
- A shape meant to sit on another needs `overlap=ok`.

Box styles (default size):

| Style | Size | For |
|---|---|---|
| `node` | 144x64 | a part, service, component or step |
| `focal` | 144x64 | the one or two things the diagram is about |
| `store` | 144x64 | something that holds data (a cylinder) |
| `cloud` | 144x64 | a hosted platform; a quiet box (tree leaves, resting states) |
| `external` | 144x64 | a person or system outside what the note is about (dashed) |
| `optional` | 144x64 | an optional or asynchronous part |
| `step` | 160x48 | a flowchart or process action |
| `terminal` | 160x48 | flowchart start or end |
| `terminal-focal` | 160x48 | the flowchart outcome that matters most |
| `decision` | 200x96 | a yes/no question, on two short lines |
| `zone` | give WxH | a labelled region drawn behind a group of boxes |
| `bar` | 8x80 | an activation bar on a sequence lifeline |
| `rect`, `ellipse`, `diamond` | give WxH | plain shapes |

## Arrows and lines

`[id:] from -> to "label" props`. `->` has a head, `<->` two, `--` none.

- `style=` picks a line style: `arrow` (default for `->`), `primary` (the main path, at most
  two), `link` (a request crossing a boundary), `return` (dashed: a reply, something optional
  or asynchronous), `relation` (`--` with square corners: trees, tables), `lifeline`, `rule`.
- An end is a shape id (`api`), a side (`api.right`, `.top`, `.bottom`, `.left`), a point on a
  side (`api.bottom@0.5`), a free point (`400,300`), a point relative to the other end
  (`+0,128`), or a point on a line (`life@176`).
- Arrows turn at right angles and go around boxes. Line boxes up so the main path is straight.
  `route=curve` draws a curve; `route=straight` a straight line.
- Lines from the same start point share a trunk: `root.bottom@0.5 -- a.top style=relation`.
- Labels are short (about 14 characters) and sit above a horizontal line. A labelled arrow
  needs a gap longer than its label: 64px between boxes holds about 8 characters, 96px about 14.
- A plain line between points: `40,120 -- 600,120 stroke=muted/0.45`.

## Text

`[id:] text "words" at x,y style=name` — `at` is the left end of the baseline; `anchor=middle`
centres it on x. Styles: `text` (12px), `h1` (a title, 26px), `caption` (9.5px), `note` (13px,
an aside), `small` (mono 9px: a path, port or command), `eyebrow` (mono 8px caps: a date or
heading). Overrides: `size=`, `color=`, `weight=600`. A diagram needs no title: the note's
caption says what it shows.

## Legend

`legend at 40,Y focal="what it means" store="what it means" primary="what it means"` — one
statement, 24px under the lowest element. Entries in the order written; for marks that aren't
styles, `dot.accent="Milestone"`, `swatch.muted="Planned"`, `line.link="Depends on"`.

## Spacing

- Pick column x and row y positions and reuse them. Boxes in a row share a top edge and height.
- Same gap between neighbours: 52–64px when a labelled arrow joins them, 40px or more otherwise.
- No arrow through a box, no arrow on another, no label touching a box or a line.
- Centre the composition: rows, branches and the legend balanced across the width.

## Worked examples

Architecture: the parts left to right on one row, stores at the end.

```fdraw
web: external "Browser" at 40,40 128x64
api: focal "Orders API" sub="REST" at 232,40
db:  store "Orders DB" at 440,40
web -> api "order" style=link
api -> db "writes" style=primary
legend at 40,128 external="Outside" focal="This service" store="Holds data" link="Request" primary="Main path"
```

Flowchart: only for real yes/no questions. The main path straight down one centre x; each
question's other answer goes sideways to a box level with the diamond, about 120px away. Label
only the answers.

```fdraw
start: terminal "Order in" at 60,40
ok:    decision "In\nstock?" at 40,112
ship:  terminal-focal "Ship it" at 60,248
wait:  step "Back-order" at 360,136
start -> ok
ok -> ship "yes" style=primary
ok -> wait "no"
legend at 40,320 terminal="Start or end" decision="Question" step="Action" primary="Main path"
```

Process: steps in order, left to right, what goes in or how long in `sub=`. No labels.

```fdraw
mix:  step "Mix" sub="flour + water" at 40,40
rest: step "Rest" sub="1 hour" at 252,40
bake: focal "Bake" sub="230 °C, 40 min" at 464,40 160x48
mix -> rest
rest -> bake style=primary
legend at 40,112 step="Step" focal="The step that matters" primary="Main path"
```

State: states the same size (`160x80 radius=8`) on one row, each transition labelled with its
event. A step back arcs over the row, dashed. The start is a filled dot. Leave 96px above the
row for the arc.

```fdraw
start:  ellipse at 40,168 16x16 fill=ink stroke=none
draft:  node "Draft" at 120,136 160x80 radius=8
review: node "In review" at 344,136 160x80 radius=8
done:   focal "Published" at 568,136 160x80 radius=8
start -> draft
draft -> review "submit"
review -> done "approve" style=primary
review.top -> draft.top "reject" style=return route=curve
legend at 40,240 node="State" focal="Outcome" primary="Main path" return="Back a step"
```

Layers: full-width bands touching, highest level on top, no arrows. The layer the note is
about is `focal`, written last.

```fdraw
ui:   rect "Interface" at 40,40 480x56 fill=ink/0.03 stroke=ink/0.15
disk: rect "Storage" at 40,152 480x56 fill=ink/0.03 stroke=ink/0.15
core: focal "Core" sub="rules and indexes" at 40,96 480x56 radius=0
legend at 40,232 focal="The layer this note is about"
```

Timeline: one axis, a dot per event, spaced by the real time between them (work out px per
month first). Labels alternate below and above the axis, centred on the event.

```fdraw
40,120 -- 600,120 stroke=muted/0.45
e1: ellipse at 76,116 8x8 fill=ink stroke=none
e2: ellipse at 316,116 8x8 fill=ink stroke=none
e3: ellipse at 554,114 12x12 fill=accent stroke=none
80,120 -- 80,176 stroke=ink/0.3
text "MAR 2026" at 80,192 style=eyebrow anchor=middle
text "Prototype" at 80,208 anchor=middle weight=600 color=ink
320,64 -- 320,120 stroke=ink/0.3
text "JUN 2026" at 320,36 style=eyebrow anchor=middle
text "Beta" at 320,52 anchor=middle weight=600 color=ink
560,120 -- 560,176 stroke=accent
text "OCT 2026" at 560,192 style=eyebrow anchor=middle color=accent
text "Launch" at 560,208 anchor=middle weight=600 color=ink
legend at 40,240 dot.ink="Event" dot.accent="Milestone"
```

Tree: root on top, one row per level 120px apart, boxes 48 high. Place the leaves first (144
wide, 40px apart), then centre each parent over its children. Lines share a trunk, no heads.

```fdraw
root: focal "Bread" at 216,40 160x48
a: cloud "Yeast" at 40,160 144x48
b: cloud "Sourdough" at 224,160 144x48
c: cloud "Flatbread" at 408,160 144x48
root.bottom@0.5 -- a.top style=relation
root.bottom@0.5 -- b.top style=relation
root.bottom@0.5 -- c.top style=relation
legend at 40,240 focal="The subject" cloud="Kind"
```

Sequence: participants in a row 224px apart, a lifeline hanging from each, messages as
horizontal arrows between points on the lifelines at the same y, 56px apart, in time order.
Replies are `return`. Lifelines end 24px after the last message.

```fdraw
app: external "App" at 40,40 144x56
api: focal "Auth API" at 264,40 144x56
app_l: app.bottom -- +0,128 style=lifeline
api_l: api.bottom -- +0,128 style=lifeline
app_l@144 -> api_l@144 "login" style=link
api_l@200 -> app_l@200 "token" style=return
legend at 40,248 external="Caller" focal="This service" link="Request" return="Reply"
```
