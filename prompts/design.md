## Design system: r1/default

<!-- Adapted from eneeware design-systems/enee-default/DESIGN.md at 61bc7a7 for the R1's screen, wheel and voice (r1os ADR-0025). -->

Apps fill the Rabbit R1's 2.88" screen: about **330 × 410 px** of content under the app's title (one px is one dp). r1os draws every app in its own look, like the rest of the R1: big, bold, lowercase type on black, buttons as dark pills, and one solid fill in the app's accent on whatever the wheel is on. Everything is large, so little fits at once: design the first screen, and put the rest below it.

### How people use an app

- **The wheel** moves the focus through the app's buttons, toggles and list-row actions in reading order, and scrolls past everything else a block at a time. **Pressing the side button** activates the focused one. Touch works too, but design for the wheel.
- **They talk instead of typing.** Holding the button while the app is open sends what they say to their agent, which knows which app is on screen and runs its actions with the details from their words: "add milk", "log a 5k in 28 minutes", "note: call mum", "spent 12 on lunch". So:
  - **No `form`, `input` or other text fields**, unless the brief explicitly asks to type in the app. Anything the user would add in their own words (a name, a note, an amount that isn't a preset, a date) is an **action with params**, e.g. `add: { params: { item: text }, do: [ … ] }`. It needs no button: the agent runs it.
  - What doesn't need words stays on screen: `quick_actions` with preset values ("+1 glass", "+250 ml"), toggles, and row actions (done, delete).
  - An empty list says what to say: `empty: Say "add milk" to add one`.
- **Values set in the app are turned, not typed or tapped through:** a goal, a tip, a size, how many, how long is a `wheel` (a click takes the wheel, turning sets it, a click keeps it). Prefer one wheel to a row of +/− buttons, and to a select of many options. A value worked out from it (a converter's result, each person's share) shows in a `stat` above it and follows as it turns.
- **Games** are one `game` block, first on the screen (a stat with the best score can follow). The wheel steers, aims or chooses; a click acts (jump, fire, drop); `down`/`up` time a press. Hold stays the agent's.
  - Draw big, simple shapes in `r1.theme`'s colours on its `bg`, the score large in a corner, and nothing finer than about 4 px.
  - Keep a round short (a minute or two) and end it with `r1.end`.
  - Save a best score with `r1.save` to a persisted state key, and record rounds with an action when the app keeps a history.

### The first screen (about 410 px tall)

Rough heights: a lone stat with its label and caption is about 100 px (a row of 2 stats, about 80); a button or a row of quick actions 50; a ghost button 40; a wheel 58 (about 130 while it's turned); a progress bar 26; a list row 56; a chart about 160; a game 360 unless its `height` says otherwise; and 14 px between blocks.

- **Open with a summary:** one `stat` or `timer`, or a `row` of 2 stats of the same kind (today · goal). Never 3: the screen is narrow.
- **Then the most common action:** `quick_actions` with 2–3 preset buttons, or one `button`. The wheel reaches it first. At most two rows of buttons before the content.
- **Then one block that fits:** a chart, or the first few list rows. The rest goes below; the wheel brings each block fully into view.
- **About four wheel stops** on the first screen: every button, wheel and row action is one.
- One chart at most. Lists show recent items (`LIMIT 20`), with row actions only for what's done often (done, delete).
- Labels and captions are a few words (they're small); values are what's big.

### Voice

- Write in sentence case: "Log a set", "This week" (r1os shows everything lowercase).
- Labels are one or two nouns: Today, This week, Goal. Buttons are short verbs or presets: Log, +1 glass, Start focus.
- Toasts confirm in a few words ("Note saved", "+20 logged"). Empty states say what will show up, or what to say to add something.

### Color and icons

- Give the app one `accent`: orange, amber, yellow, red, pink, purple, indigo, blue, cyan, teal, green or lime. Never write raw colors.
- Buttons: `primary` (the accent's color) for the main action, `secondary` for the others, `ghost` for a quiet one like "Undo last". The accent's fill is reserved for the wheel's focus.
- Icons are Phosphor names, such as `barbell`, `drop`, `coffee`, `trash`, `check` or `timer`.
- Use the `danger` button variant only for destructive actions, and give those actions a `confirm:`.
