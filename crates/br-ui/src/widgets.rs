//! What each of BR's controls runs: every button, slider, tile and field BR draws has a widget id
//! (`app.widgets`, `br.inspect`), and here the command it runs, so an agent can do through the
//! control channel whatever a click does, and the command search lists it. A `*` in a pattern
//! stands for one or more characters; in the command it is replaced by what it stood for.
//! Areas that are not controls (a popover's frame, the toast) are in [`AREAS`].

/// (widget pattern, commands it runs separated by `|`), the first match counts.
pub const CONTROLS: &[(&str, &str)] = &[
    // the header
    ("page.*", "br.ui.page.*"),
    ("video.export", "br.ui.popover.export"),
    ("video.format", "br.ui.popover.format"),
    // the command search
    ("palette.search", "br.ui.palette"),
    ("palette.cmd.*", "*"),
    ("palette.row.*", "br.palette.run"),
    ("palette.field.*", "br.palette.run"),
    ("palette.run", "br.palette.run"),
    ("palette.back", "br.ui.palette"),
    // the start screen
    ("start.template.*", "br.video.template.new"),
    ("start.recent.*", "file.open"),
    ("start.add", "br.video.add"),
    ("start.open", "file.open"),
    ("start.demo", "file.openDemoProject"),
    ("start.skip", "br.ui.start.skip"),
    ("start.*", "br.video.new"),
    // the tool row
    ("cut.add", "br.video.add"),
    ("cut.text", "br.video.text"),
    ("cut.captions.cancel", "br.video.cancel"),
    ("cut.captions", "br.ui.popover.captions"),
    ("cut.music", "br.video.music"),
    ("cut.filter", "br.ui.popover.filter"),
    ("cut.transition", "br.video.transition"),
    ("cut.split", "br.video.split"),
    ("cut.delete", "br.video.delete"),
    ("cut.speed", "br.ui.popover.speed"),
    ("cut.detach", "br.video.detachAudio"),
    ("cut.undo", "edit.undo"),
    ("cut.redo", "edit.redo"),
    // the popovers
    ("captions.auto", "br.video.autoCaptions"),
    ("captions.edit", "br.ui.captions.edit"),
    ("cut.filter.more", "br.ui.page.farbe"),
    ("captions.file", "br.video.captions"),
    ("filter.*", "br.video.filter"),
    ("speed.curve.*", "br.video.curve"),
    ("speed.freeze", "br.video.freeze"),
    ("speed.reverse", "br.video.reverse"),
    ("speed.*", "br.video.speed"),
    ("vformat.bg.*", "br.video.background"),
    ("vformat.*", "br.video.format"),
    ("vexport.cancel", "br.video.cancel"),
    ("vexport.page", "br.ui.page.ausgabe"),
    ("vexport.quick", "br.ui.export.quick"),
    ("vexport.*", "br.video.export"),
    ("vpreset.*", "br.video.export"),
    // the timeline
    ("tl.beats", "br.video.beats"),
    ("tl.magnet", "br.video.magnet"),
    ("tl.linkage", "br.video.linkage"),
    ("tl.view.full", "br.ui.timeline.profi"),
    ("tl.zoom.in", "br.ui.timeline.zoomIn"),
    ("tl.zoom.out", "br.ui.timeline.zoomOut"),
    ("tl.zoom.fit", "br.ui.timeline.zoomFit"),
    ("tl.ruler", "playhead.set"),
    ("tl.clip.*", "timeline.select|br.video.move|br.video.trim"),
    // the inspector
    ("insp.level.*", "br.ui.inspector.*"),
    ("insp.tab.*", "br.ui.inspector"),
    ("insp.voll.*", "br.ui.inspector"),
    ("insp.section.*.key", "br.video.keyframe"),
    ("insp.row.*.key", "br.video.keyframe"),
    ("insp.row.*.reset", "br.video.set"),
    ("insp.row.*", "br.video.set"),
    ("insp.fx.*.key", "br.video.keyframe"),
    ("insp.fx.*.enable", "br.video.effect"),
    ("insp.fx.*.remove", "br.video.effect"),
    ("insp.fx.*.reset", "br.video.set|br.video.effect"),
    ("insp.fx.*", "br.video.set"),
    ("insp.add.search", "br.video.schema"),
    ("insp.add.*", "br.video.effect"),
    ("insp.add", "br.video.effect"),
    // the asset bar
    ("assets.tab.*", "br.ui.assets.*"),
    ("assets.*.search", "br.ui.assets"),
    ("assets.*.import", "file.import"),
    ("assets.media.*", "br.video.place"),
    ("assets.audio.*", "br.video.place"),
    ("assets.text.default", "br.video.text"),
    ("assets.text.*", "graphics.template.apply"),
    ("assets.effect.*", "br.video.effect"),
    ("assets.transition.*", "br.video.transition"),
    ("assets.look.*", "br.video.filter"),
    ("assets.template.fill", "br.video.template.fill"),
    ("assets.template.save", "br.video.template.save"),
    ("assets.template.*", "br.video.template.new"),
    ("assets.profi.*", "br.ui.assets.profi"),
    // the Foto page
    ("tool.*", "br.ui.photo.*"),
    ("look.amount", "br.look"),
    ("look.*", "br.look"),
    ("format.*", "br.crop"),
    ("preset.*", "br.export"),
    ("export.*", "br.export"),
    ("export", "br.ui.photo.export"),
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

/// The commands a control runs, or `None` for an area or an id BR does not know.
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
        assert_eq!(commands_of("page.foto").unwrap(), ["br.ui.page.foto"]);
        assert_eq!(commands_of("palette.cmd.edit.undo").unwrap(), ["edit.undo"]);
        assert_eq!(commands_of("start.9:16").unwrap(), ["br.video.new"]);
        assert_eq!(commands_of("start.template.story").unwrap(), ["br.video.template.new"]);
        assert_eq!(commands_of("insp.row.transform.scale.key").unwrap(), ["br.video.keyframe"]);
        assert_eq!(commands_of("insp.add").unwrap(), ["br.video.effect"]);
        assert_eq!(commands_of("tl.clip.12").unwrap().len(), 3);
        assert!(commands_of("popover.speed").is_none() && is_area("popover.speed"));
        assert!(commands_of("gibtsnicht").is_none());
    }
}
