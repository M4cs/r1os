Change the app below as the user asks, following the builder reference in your system prompt.

1. Change only what the request needs. Keep everything else as it is: the app `id`, table and column names, query and action names, settings and their defaults, and the order of the views.
2. Tables and columns hold the user's data. Remove one only when the request asks to remove that data, with `name: null` in the patch: it's deleted with its data (an undo brings it back). Never rename one, since that deletes its data too: change its label in the views instead. Give each new column a default or make it optional.
3. Follow the layout rules for anything you add or move. The screen is small: keep the first screen to a summary, its actions and one more block.
4. Don't add forms or text fields unless the request asks to type in the app. Something the user adds in their own words is an action with params; they say it while the app is open, and their agent runs it.
5. Keep the checks passing, and add one if the request adds behavior.
6. A reminder is a trigger with `notify` (and the `schedule` and `notify` permissions); keep reminders rare and tunable with settings.
7. When the request asks for facts from the world that the app has the user type in (the weather, a price, the news), fetch them with a service and `net.http` (the builder reference's services). Keep the tables that hold what the user already entered unless the request says to drop them.

Start with one short sentence for the user saying what you changed, in plain words and the past tense: the chat shows it as your reply, and it may be read aloud. Then exactly one ```enee-patch block of only what changes (the builder reference's Patches section), and nothing else. For example, "show this week's total" for a pushup tracker:

Added this week's total under today's.

```enee-patch
queries:
  week: SELECT coalesce(sum(reps), 0) AS total FROM sets WHERE is_this_week(at)
views:
  main:
    - stat: { label: Today, value: "{{today.total}}", caption: "{{today.n}} sets · goal {{settings.daily_goal}}" }
    - stat: { label: This week, value: "{{week.total}}" }
    - quick_actions:
        - { label: "+10", action: log, params: { reps: 10 } }
        - { label: "+20", action: log, params: { reps: 20 } }
    - list:
        data: recent
        title: "{{row.reps}} reps"
        caption: "{{row.at | time}}"
        row_actions: [ { icon: trash, label: Delete, action: remove, params: { id: "{{row.id}}" } } ]
        empty: No sets yet
```

Only when the request rewrites most of the app, give one ```enee block of the complete spec instead of the patch, after the same sentence.

The request and the app's current spec follow.
