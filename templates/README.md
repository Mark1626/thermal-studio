# Templates

The official list of templates that Thermal Studio can add (**Templates → Add…**). The apps read
[`index.json`](index.json) from this folder on GitHub; each template is the `.md` file named after its id.

The templates built into the apps are in [`crates/thermal-templates/templates`](../crates/thermal-templates/templates).

## Adding a template

1. Write it, following the [template format](../crates/thermal-templates/README.md). Name the file after its `id`,
   such as `calendar-tracker.md`.
2. Try it: `cargo run -p thermal-cli -- template templates/calendar-tracker.md --set title=Walk -o tracker.png`
3. Update the list: `cargo run -p thermal-cli -- index-templates templates`
4. Check it: `cargo test -p thermal-templates` fails if a template doesn't load or `index.json` is out of date.

When you change a template, raise its `version`, so people who already added it are offered the update.
