import { useEffect, useRef, useCallback } from "react";
import { EditorState, Compartment, type Extension, StateEffect, StateField } from "@codemirror/state";
import {
  EditorView,
  lineNumbers,
  highlightActiveLineGutter,
  keymap,
  drawSelection,
  Decoration,
  type DecorationSet,
} from "@codemirror/view";
import { foldGutter, foldKeymap, bracketMatching } from "@codemirror/language";
import { defaultKeymap } from "@codemirror/commands";
import { search, searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { useThemeStore } from "../../lib/theme";
import { createThemeExtensions } from "./cm-theme";
import { loadLanguageExtension } from "./cm-languages";

export interface CodeViewerProps {
  content: string;
  language: string;
  line?: number;
  wordWrap?: boolean;
}

const highlightLineEffect = StateEffect.define<number | null>();

const lineHighlightField = StateField.define<DecorationSet>({
  create() {
    return Decoration.none;
  },
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(highlightLineEffect)) {
        if (effect.value == null) return Decoration.none;
        const docLine = tr.state.doc.line(Math.min(Math.max(1, effect.value), tr.state.doc.lines));
        return Decoration.set([
          Decoration.line({ class: "cm-line-highlight" }).range(docLine.from),
        ]);
      }
    }
    return deco;
  },
  provide: (field) => EditorView.decorations.from(field),
});

function scrollToLine(view: EditorView, lineNumber: number) {
  const clamped = Math.min(Math.max(1, lineNumber), view.state.doc.lines);
  const docLine = view.state.doc.line(clamped);
  view.dispatch({
    effects: [
      highlightLineEffect.of(clamped),
      EditorView.scrollIntoView(docLine.from, { y: "center" }),
    ],
  });
}

function fadeLineHighlight(
  view: EditorView,
  timerRef: React.MutableRefObject<number | undefined>,
) {
  const deco = view.state.field(lineHighlightField, false);
  if (!deco || deco.size === 0) return;

  const dom = view.dom.querySelector(".cm-line-highlight");
  if (dom instanceof HTMLElement) {
    dom.classList.add("cm-line-highlight-fade");
  }

  if (timerRef.current != null) window.clearTimeout(timerRef.current);
  timerRef.current = window.setTimeout(() => {
    timerRef.current = undefined;
    if (!view.dom.isConnected) return;
    view.dispatch({ effects: highlightLineEffect.of(null) });
  }, 3000);
}

export function CodeViewer({ content, language, line, wordWrap = false }: CodeViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const langCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  const wrapCompartment = useRef(new Compartment());
  const resolved = useThemeStore((s) => s.resolved);
  const lineHandledRef = useRef<string | undefined>(undefined);
  const fadeTimerRef = useRef<number | undefined>(undefined);

  const buildExtensions = useCallback((): Extension[] => {
    return [
      lineNumbers(),
      highlightActiveLineGutter(),
      foldGutter(),
      lineHighlightField,
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      drawSelection(),
      bracketMatching(),
      search({ top: true }),
      highlightSelectionMatches(),
      keymap.of([...searchKeymap, ...foldKeymap, ...defaultKeymap]),
      themeCompartment.current.of(createThemeExtensions()),
      langCompartment.current.of([]),
      wrapCompartment.current.of(wordWrap ? EditorView.lineWrapping : []),
    ];
  }, [wordWrap]);

  // Create / destroy EditorView
  useEffect(() => {
    const parent = containerRef.current;
    if (!parent) return;

    const view = new EditorView({
      state: EditorState.create({
        doc: content,
        extensions: buildExtensions(),
      }),
      parent,
    });
    viewRef.current = view;
    lineHandledRef.current = undefined;

    return () => {
      if (fadeTimerRef.current != null) {
        window.clearTimeout(fadeTimerRef.current);
      }
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- initial mount only
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== content) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: content },
      });
      lineHandledRef.current = undefined;
      view.scrollDOM.scrollTop = 0;
    }
  }, [content]);

  // Load language extension
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    let cancelled = false;
    void loadLanguageExtension(language).then((exts) => {
      if (cancelled || !viewRef.current) return;
      viewRef.current.dispatch({
        effects: langCompartment.current.reconfigure(exts),
      });
    });

    return () => {
      cancelled = true;
    };
  }, [language]);

  // Theme follow
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: themeCompartment.current.reconfigure(createThemeExtensions()),
    });
  }, [resolved]);

  // Word wrap toggle
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: wrapCompartment.current.reconfigure(wordWrap ? EditorView.lineWrapping : []),
    });
  }, [wordWrap]);

  // Jump to line
  useEffect(() => {
    const view = viewRef.current;
    if (!view || line == null || line < 1) return;

    const token = `${content.length}:${language}:${line}`;
    if (lineHandledRef.current === token) return;

    requestAnimationFrame(() => {
      if (!viewRef.current) return;
      scrollToLine(viewRef.current, line);
      lineHandledRef.current = token;

      if (fadeTimerRef.current != null) {
        window.clearTimeout(fadeTimerRef.current);
      }
      fadeTimerRef.current = window.setTimeout(() => {
        if (viewRef.current) fadeLineHighlight(viewRef.current, fadeTimerRef);
      }, 50);
    });
  }, [line, content, language]);

  return (
    <div
      ref={containerRef}
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
        overflow: "hidden",
      }}
    />
  );
}
