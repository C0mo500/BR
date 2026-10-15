//! What each of shnyp's controls runs: every button, slider, tile and field shnyp draws has a widget id
//! (`app.widgets`, `shnyp.inspect`), and here the command it runs, so an agent can do through the
//! control channel whatever a click does, and the command search lists it. A `*` in a pattern
//! stands for one or more characters; in the command it is replaced by what it stood for.
//! Areas that are not controls (a popover's frame, the toast) are in [`AREAS`].

/// (widget pattern, commands it runs separated by `|`), the first match counts.
pub const CONTROLS: &[(&str, &str)] = &[
    // the header
    ("page.*", "shnyp.ui.page.*"),
    ("video.export", "shnyp.ui.popover.export"),
    ("video.format", "shnyp.ui.popover.format"),
    // the command search
    ("palette.search", "shnyp.ui.palette"),
    ("palette.cmd.*", "*"),
    ("palette.row.*", "shnyp.palette.run"),
    ("palette.field.*", "shnyp.palette.run"),
    ("palette.run", "shnyp.palette.run"),
    ("palette.back", "shnyp.ui.palette"),
    // the start screen
    ("start.template.*", "shnyp.video.template.new"),
    ("start.recent.*", "file.open"),
    ("start.add", "shnyp.video.add"),
    ("start.open", "file.open"),
    ("start.demo", "file.openDemoProject"),
    ("start.skip", "shnyp.ui.start.skip"),
    ("start.*", "shnyp.video.new"),
    // the tool row
    ("cut.add", "shnyp.video.add"),
    ("cut.text", "shnyp.video.text"),
    ("cut.captions.cancel", "shnyp.video.cancel"),
    ("cut.captions", "shnyp.ui.popover.captions"),
    ("cut.music", "shnyp.video.music"),
    ("cut.filter", "shnyp.ui.popover.filter"),
    ("cut.transition", "shnyp.video.transition"),
    ("cut.split", "shnyp.video.split"),
    ("cut.delete", "shnyp.video.delete"),
    ("cut.speed", "shnyp.ui.popover.speed"),
    ("cut.detach", "shnyp.video.detachAudio"),
    ("cut.undo", "edit.undo"),
    ("cut.redo", "edit.redo"),
    // the popovers
    ("captions.auto", "shnyp.video.autoCaptions"),
    ("captions.edit", "shnyp.ui.captions.edit"),
    ("cut.filter.more", "shnyp.ui.page.farbe"),
    ("captions.file", "shnyp.video.captions"),
    ("filter.*", "shnyp.video.filter"),
    ("speed.curve.*", "shnyp.video.curve"),
    ("speed.freeze", "shnyp.video.freeze"),
    ("speed.reverse", "shnyp.video.reverse"),
    ("speed.*", "shnyp.video.speed"),
    ("vformat.bg.*", "shnyp.video.background"),
    ("vformat.*", "shnyp.video.format"),
    ("vexport.cancel", "shnyp.video.cancel"),
    ("vexport.page", "shnyp.ui.page.ausgabe"),
    ("vexport.quick", "shnyp.ui.export.quick"),
    ("vexport.*", "shnyp.video.export"),
    ("vpreset.*", "shnyp.video.export"),
    // the timeline
    ("tl.beats", "shnyp.video.beats"),
    ("tl.magnet", "shnyp.video.magnet"),
    ("tl.linkage", "shnyp.video.linkage"),
    ("tl.view.full", "shnyp.ui.timeline.profi"),
    ("tl.zoom.in", "shnyp.ui.timeline.zoomIn"),
    ("tl.zoom.out", "shnyp.ui.timeline.zoomOut"),
    ("tl.zoom.fit", "shnyp.ui.timeline.zoomFit"),
    ("tl.ruler", "playhead.set"),
    ("tl.clip.*", "timeline.select|shnyp.video.move|shnyp.video.trim"),
    // the inspector
    ("insp.level.*", "shnyp.ui.inspector.*"),
    ("insp.tab.*", "shnyp.ui.inspector"),
    ("insp.voll.*", "shnyp.ui.inspector"),
    ("insp.section.*.key", "shnyp.video.keyframe"),
    ("insp.row.*.key", "shnyp.video.keyframe"),
    ("insp.row.*.reset", "shnyp.video.set"),
    ("insp.row.*", "shnyp.video.set"),
    ("insp.fx.*.key", "shnyp.video.keyframe"),
    ("insp.fx.*.enable", "shnyp.video.effect"),
    ("insp.fx.*.remove", "shnyp.video.effect"),
    ("insp.fx.*.reset", "shnyp.video.set|shnyp.video.effect"),
    ("insp.fx.*", "shnyp.video.set"),
    ("insp.add.search", "shnyp.video.schema"),
    ("insp.add.*", "shnyp.video.effect"),
    ("insp.add", "shnyp.video.effect"),
    // the asset bar
    ("assets.tab.*", "shnyp.ui.assets.*"),
    ("assets.*.search", "shnyp.ui.assets"),
    ("assets.*.import", "file.import"),
    ("assets.media.*", "shnyp.video.place"),
    ("assets.audio.*", "shnyp.video.place"),
    ("assets.text.default", "shnyp.video.text"),
    ("assets.text.*", "graphics.template.apply"),
    ("assets.effect.*", "shnyp.video.effect"),
    ("assets.transition.*", "shnyp.video.transition"),
    ("assets.look.*", "shnyp.video.filter"),
    ("assets.template.fill", "shnyp.video.template.fill"),
    ("assets.template.save", "shnyp.video.template.save"),
    ("assets.template.*", "shnyp.video.template.new"),
    ("assets.profi.*", "shnyp.ui.assets.profi"),
    // the Foto page
    ("tool.*", "shnyp.ui.photo.*"),
    ("look.amount", "shnyp.look"),
    ("look.*", "shnyp.look"),
    ("format.*", "shnyp.crop"),
    ("preset.*", "shnyp.export"),
    ("export.*", "shnyp.export"),
    ("export", "shnyp.ui.photo.export"),
];

/// Ids that are areas, not controls.
pub const AREAS: &[&str] = &["popover.*", "palette", "toast"];

/// What `pattern` matches in `id`: the text `*` stood for ("" without a `*`).
fn matches<'a>(pattern: &str, id: &'a str) -> Option<&'a str> {
    match pattern.split_once('*') {
        None => (pattern == id).then_some(""),
        Some((head, tail)) => {
            let rest = id.strip_prefix(head)?;
            let mid = rest.strip_suffix(tail)?;
            (!mid.is_empty()).then_some(mid)
        }
    }
}

/// The commands a control runs, or `None` for an area or an id shnyp does not know.
pub fn commands_of(id: &str) -> Option<Vec<String>> {
    CONTROLS.iter().find_map(|(p, c)| matches(p, id).map(|m| c.split('|').map(|c| c.replace('*', m)).collect()))
}

/// Whether `id` is an area (no command).
pub fn is_area(id: &str) -> bool {
    AREAS.iter().any(|p| matches(p, id).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_name_their_commands() {
        assert_eq!(commands_of("page.foto").unwrap(), ["shnyp.ui.page.foto"]);
        assert_eq!(commands_of("palette.cmd.edit.undo").unwrap(), ["edit.undo"]);
        assert_eq!(commands_of("start.9:16").unwrap(), ["shnyp.video.new"]);
        assert_eq!(commands_of("start.template.story").unwrap(), ["shnyp.video.template.new"]);
        assert_eq!(commands_of("insp.row.transform.scale.key").unwrap(), ["shnyp.video.keyframe"]);
        assert_eq!(commands_of("insp.add").unwrap(), ["shnyp.video.effect"]);
        assert_eq!(commands_of("tl.clip.12").unwrap().len(), 3);
        assert!(commands_of("popover.speed").is_none() && is_area("popover.speed"));
        assert!(commands_of("gibtsnicht").is_none());
    }
}
