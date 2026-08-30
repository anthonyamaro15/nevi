# Nevi

A Neovim-inspired terminal editor where existing vim/nvim muscle memory works unchanged. Terms below are the canonical vocabulary for its editing and rendering model.

## Language

### Mouse & scrolling

**Mouse capture**:
The terminal handing mouse events to nevi instead of acting on them itself (escape-code mouse reporting). While active, terminal-native selection needs the terminal's bypass key (Option in iTerm2).
_Avoid_: mouse mode, mouse reporting

**Viewport scroll**:
Moving a pane's visible window over the buffer without moving the cursor; the cursor is dragged along only when it would leave the visible area. What the wheel and `<C-e>`/`<C-y>` do.
_Avoid_: cursor scroll, paging

**Scroll target**:
The pane the pointer is over — wheel events scroll it, not the focused pane.
_Avoid_: active pane (that's the focused one)

**Wheel tick**:
One discrete scroll-wheel notch as delivered by the terminal; scrolls the target 3 lines vertically.

### Panes & rendering

**Pane**:
One rectangular editor window in a split layout, owning its own cursor, viewport offset, and buffer reference. The Editor mirrors the active pane's state.
_Avoid_: window, split (a split is the act, the pane is the thing)

**Viewport offset**:
The buffer line at the top of a pane's visible area.
_Avoid_: scroll position, top line

**Damage**:
The per-frame record of what must repaint — full frame, or specific rows. Any viewport offset change is full damage.
_Avoid_: dirty flags
