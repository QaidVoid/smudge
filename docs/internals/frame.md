# Anatomy of a frame

<p class="lead">Every frame, windowed or headless, runs the same pipeline. Each stage does work only for what changed, and the output is a display list plus the rectangles of pixels that differ from the last frame.</p>

```text
 messages ─► flush ─► layout ─► paint ─► display list ─► damage ─► raster ─► present
 timers      and      only if   only     replays          old vs    only the   only the
 tick        apply    a size    marked   cached ops       new       damaged    damaged
                      changed   nodes                     bounds    rects      region
```

## 1. Messages, timers and the tick

The frame first delivers the values other threads sent through the app's channels. Each channel's handler runs inside the owner that created the channel, and delivers at most what the queue held when the frame reached it. The timers that are due on the app's clock fire next. Last, if the app installed a tick with `App::set_tick`, it runs with the frame's time. Animations write signals here, and so can the handlers and timers before it.

## 2. Flush and apply

Signal writes since the last frame have marked the bindings that read them. The flush runs those bindings in the order they were woken. A binding does not touch the tree directly: it queues an update, such as "set this label's text" or "set this panel's background".

The frame then applies the queued updates to the tree. Each update marks its node with what it invalidates: a text or size change marks layout, a color change marks paint only. Applying can wake more bindings, so flush and apply repeat until nothing is left.

## 3. Layout

If any node was marked for layout, the tree is laid out with flexbox and block rules. Nodes whose size inputs did not change keep their cached measurements, so a text change in one label measures that label and the containers around it.

After layout, the frame computes each node's rectangle in window coordinates. A node whose rectangle moved is marked for paint; a node that stayed put is not.

A scroll is special. Widgets paint in coordinates that leave out their ancestors' scroll offsets, so a scrolled node's paint does not change when it moves. Only its window rectangle changes, and the scroll container wraps its content in an offset. Scrolling a thousand rows therefore repaints none of them.

## 4. Paint

Each node marked for paint asks its widget to paint again, which produces a short list of operations: fill this rounded rectangle, stroke this border, draw these glyphs. Every other node keeps the operations it produced last time. In a typical interaction, one to a few nodes paint per frame.

## 5. Display list

The frame walks the tree and concatenates the nodes' cached operations, in paint order, into the display list, with clip and offset items around the subtrees that clip or scroll. The display list is the whole frame, described once, for any renderer.

## 6. Damage

Each node that painted again reports two rectangles: where it drew last frame and where it draws now. A node that moved without repainting, such as a scrolled row, reports its old and new positions. Together these are the frame's damage, in logical pixels.

The damage is converted to physical pixels and merged: two rectangles whose union costs little more than the pair are replaced by the union, because a renderer pays a fixed cost per rectangle. When the damage covers most of the surface, or stays scattered across too many rectangles, it collapses to one full-surface rectangle, because repainting everything is then cheaper.

When an app animates with a tick and a frame comes out with no damage, the window skips the present, because the screen already shows that picture. A frame the window system asked for, for example after the window was uncovered, presents in full to restore it.

## 7. Raster

The CPU renderer keeps last frame's pixels. For each damaged rectangle, it clips the display list to that rectangle, skips every operation that does not intersect it, and draws the rest. Pixels outside the damage are never touched, so the result is identical to drawing the whole frame, at the cost of the damaged area. Glyphs are rasterized once into a cache and reused.

The GPU renderer draws the whole display list every frame. Damage still decides whether a frame happens at all.

## 8. Present

On the desktop, the window asks the platform which of its buffers it is about to reuse and how old that buffer's contents are. When the platform reports the age, the window copies only the damaged rectangles into the buffer and tells the compositor which region changed. When it cannot, the whole surface is copied.

## What a small change costs

A press on the counter's button, from the [quick start](/guide/quick-start):

| Stage | Work |
|---|---|
| Flush | The label's text closure runs once. |
| Layout | The label measures its new text. The column around it checks whether its size changed. |
| Paint | The label and the button, whose pressed color changed. |
| Damage | Two small rectangles. |
| Raster and present | Those rectangles, a few thousand pixels. |

The rest of the window does not run any code at all.
