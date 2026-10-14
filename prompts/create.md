Build the app in the brief below as an EneeSpec for the R1, following the builder reference in your system prompt.

1. Pick the archetype and the smallest data model that covers the brief.
2. Write the queries the UI reads. Put totals, groupings and date logic in SQL, using the host helpers.
3. Write actions for every change the user makes: add, edit, remove and undo.
4. Build one view from blocks, following the design system. The screen is small and the type is big, so plan the first screen (about 410 px):
   - open with a short summary: one stat, or a row of 2;
   - read values with `{{query.column}}`;
   - put one-tap logging in quick actions right after the summary, since the wheel reaches them first;
   - then one chart or a list (`data: <query>`).
   - **No forms or text fields** unless the brief asks to type in the app. Whatever the user adds in their own words is an action with params that their agent runs when they say it ("add milk"); an empty list says what to say.
5. Add settings for numbers the user might tune (goals, sizes).
6. When the brief needs facts from the world (weather, prices, rates, news), fetch them with a service and `net.http` rather than asking the user to type them in. When it asks for reminders, use a trigger with `notify` (and `schedule`). Opening a link is an `open_url` step (`open.url`), copying text a `copy` step (`clipboard.write`). Never write any other permission: if the brief asks for something else the R1 doesn't run yet (launching apps, reading the clipboard), build the rest and leave that part out.
7. Add one to three checks that prove the core behavior.

Reply with exactly one ```enee block containing the full spec, and nothing else.

The brief follows.
