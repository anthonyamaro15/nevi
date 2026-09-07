use super::OracleCase;

/// Editing cases verify both the resulting text and Vim's final cursor/mode.
/// Yank behavior is made observable by pasting the captured register contents.
pub(super) const EDITING_CASES: &[OracleCase] = &[
    OracleCase {
        name: "delete first char on second line",
        initial_text: "alpha\nbeta\n",
        keys: "j0x",
    },
    OracleCase {
        name: "append punctuation at line end",
        initial_text: "alpha\n",
        keys: "A!<Esc>",
    },
    OracleCase {
        name: "delete current line",
        initial_text: "alpha\nbeta\n",
        keys: "dd",
    },
    OracleCase {
        name: "counted char delete",
        initial_text: "abcdef\n",
        keys: "4x",
    },
    OracleCase {
        name: "counted line delete",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2dd",
    },
    OracleCase {
        name: "delete to line end",
        initial_text: "alpha beta\n",
        keys: "wD",
    },
    OracleCase {
        name: "insert before cursor",
        initial_text: "alpha\n",
        keys: "iX<Esc>",
    },
    OracleCase {
        name: "append after cursor",
        initial_text: "alpha\n",
        keys: "aX<Esc>",
    },
    OracleCase {
        name: "open line below",
        initial_text: "alpha\n",
        keys: "ochild<Esc>",
    },
    OracleCase {
        name: "open line above",
        initial_text: "alpha\n",
        keys: "Oparent<Esc>",
    },
    OracleCase {
        name: "delete word",
        initial_text: "alpha beta\n",
        keys: "dw",
    },
    OracleCase {
        name: "delete enter motion",
        initial_text: "zero\n    one\n  two\nthree\n",
        keys: "d<CR>",
    },
    OracleCase {
        name: "change inner word",
        initial_text: "alpha beta\n",
        keys: "ciwdone<Esc>",
    },
    OracleCase {
        name: "change current line",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>",
    },
    OracleCase {
        name: "change current line register shape",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>p",
    },
    OracleCase {
        name: "counted line change",
        initial_text: "  alpha beta\nsecond\nthird\n",
        keys: "2ccreplacement<Esc>",
    },
    OracleCase {
        name: "undo indented line change",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>u",
    },
    OracleCase {
        name: "redo indented line change",
        initial_text: "  alpha beta\nsecond\n",
        keys: "ccreplacement<Esc>u<C-r>",
    },
    OracleCase {
        name: "change to line end",
        initial_text: "alpha beta gamma\n",
        keys: "wCreplaced<Esc>",
    },
    OracleCase {
        name: "change to line end register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wCreplaced<Esc>p",
    },
    OracleCase {
        name: "counted change to line end",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2Creplaced<Esc>",
    },
    OracleCase {
        name: "counted change to line end past eof",
        initial_text: "alpha\n",
        keys: "2C",
    },
    OracleCase {
        name: "yank current line",
        initial_text: "alpha\nbeta\n",
        keys: "yyp",
    },
    OracleCase {
        name: "linewise yank without final newline",
        initial_text: "abc",
        keys: "yypp",
    },
    OracleCase {
        name: "linewise paste does not expose trailing newline as a line",
        initial_text: "",
        keys: "iabc<Esc>yypj",
    },
    OracleCase {
        name: "yank to line end",
        initial_text: "alpha\nbeta\n",
        keys: "YP",
    },
    OracleCase {
        name: "counted yank to line end",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2YP",
    },
    OracleCase {
        name: "counted yank to line end past eof",
        initial_text: "alpha\n",
        keys: "x2Yp",
    },
    OracleCase {
        name: "paste after linewise yank",
        initial_text: "alpha\nbeta\n",
        keys: "ddp",
    },
    OracleCase {
        name: "paste before linewise yank",
        initial_text: "alpha\nbeta\n",
        keys: "ddP",
    },
    OracleCase {
        name: "delete to word end",
        initial_text: "alpha beta gamma\n",
        keys: "wde",
    },
    OracleCase {
        name: "delete to word end register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wdep",
    },
    OracleCase {
        name: "delete to previous word start",
        initial_text: "alpha beta gamma\n",
        keys: "wdb",
    },
    OracleCase {
        name: "delete to previous word start register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wdbp",
    },
    OracleCase {
        name: "delete with line-end motion",
        initial_text: "alpha beta gamma\n",
        keys: "wd$",
    },
    OracleCase {
        name: "counted delete with line-end motion",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2d$",
    },
    OracleCase {
        name: "counted delete with line-end motion past eof",
        initial_text: "alpha\n",
        keys: "2d$",
    },
    OracleCase {
        name: "counted delete line-end register",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2d$p",
    },
    OracleCase {
        name: "change around word",
        initial_text: "alpha beta gamma\n",
        keys: "wcawdone<Esc>",
    },
    OracleCase {
        name: "change around word register shape",
        initial_text: "alpha beta gamma\n",
        keys: "wcawdone<Esc>p",
    },
    OracleCase {
        name: "counted change around word before operator",
        initial_text: "alpha beta gamma delta\n",
        keys: "w2cawdone<Esc>",
    },
    OracleCase {
        name: "counted change around word after operator",
        initial_text: "alpha beta gamma delta\n",
        keys: "wc2awdone<Esc>",
    },
    OracleCase {
        name: "counted line yank",
        initial_text: "alpha\nbeta\ngamma\ndelta\n",
        keys: "3yyp",
    },
    OracleCase {
        name: "counted linewise paste",
        initial_text: "alpha\nbeta\n",
        keys: "dd2p",
    },
    OracleCase {
        name: "undo counted linewise paste",
        initial_text: "alpha\nbeta\n",
        keys: "dd2pu",
    },
    OracleCase {
        name: "redo counted linewise paste",
        initial_text: "alpha\nbeta\n",
        keys: "dd2pu<C-r>",
    },
    OracleCase {
        name: "undo counted linewise paste before",
        initial_text: "alpha\nbeta\n",
        keys: "dd2Pu",
    },
    OracleCase {
        name: "counted multiline linewise paste",
        initial_text: "alpha\nbeta\ngamma\ndelta\n",
        keys: "3yy2p",
    },
    OracleCase {
        name: "counted empty linewise paste",
        initial_text: "\nalpha\n",
        keys: "yy2p",
    },
    OracleCase {
        name: "counted characterwise paste",
        initial_text: "abcd\n",
        keys: "x2p",
    },
    OracleCase {
        name: "undo counted characterwise paste",
        initial_text: "abcd\n",
        keys: "x2pu",
    },
    OracleCase {
        name: "redo counted characterwise paste",
        initial_text: "abcd\n",
        keys: "x2pu<C-r>",
    },
    OracleCase {
        name: "counted characterwise paste before",
        initial_text: "abcd\n",
        keys: "x2P",
    },
    OracleCase {
        name: "counted characterwise paste after and move",
        initial_text: "abcd\n",
        keys: "x2gp",
    },
    OracleCase {
        name: "multiline characterwise paste after and move",
        initial_text: "alpha\nbeta\ngamma\n",
        keys: "2Ygp",
    },
    OracleCase {
        name: "counted characterwise paste before and move",
        initial_text: "abcd\n",
        keys: "x2gP",
    },
    // Fuzz-found: J/gJ on the last line must be a no-op. The old guard
    // counted the trailing rope sentinel as a joinable line and silently
    // deleted the file's final newline.
    OracleCase {
        name: "join on last line is a no-op",
        initial_text: "alpha\nbeta\n",
        keys: "GJ",
    },
    OracleCase {
        name: "join without space on last line is a no-op",
        initial_text: "alpha\nbeta\n",
        keys: "GgJ",
    },
    // No-EOL buffers must behave like Neovim ('fixendofline' default):
    // the buffer gains the missing final newline on load.
    OracleCase {
        name: "enter in insert at eof without trailing newline",
        initial_text: "abc",
        keys: "A<CR><Esc>",
    },
    OracleCase {
        name: "join on last line without trailing newline",
        initial_text: "alpha\nbeta",
        keys: "GJ",
    },
    // Case-changing operators: gu/gU/g~ with a motion, the doubled line
    // forms, and the ~ quick toggle.
    OracleCase {
        name: "uppercase to word end",
        initial_text: "hello world\n",
        keys: "gUw",
    },
    OracleCase {
        name: "lowercase to word end",
        initial_text: "HELLO World\n",
        keys: "guw",
    },
    OracleCase {
        name: "toggle case to word end",
        initial_text: "Hello world\n",
        keys: "g~w",
    },
    OracleCase {
        name: "uppercase entire line",
        initial_text: "hello World\nnext\n",
        keys: "gUU",
    },
    OracleCase {
        name: "lowercase entire line",
        initial_text: "HELLO World\nnext\n",
        keys: "guu",
    },
    OracleCase {
        name: "toggle case entire line",
        initial_text: "Hello World\nnext\n",
        keys: "g~~",
    },
    OracleCase {
        name: "toggle case single char",
        initial_text: "hello\n",
        keys: "~",
    },
    OracleCase {
        name: "counted toggle case",
        initial_text: "hello\n",
        keys: "3~",
    },
    // Substitute family: X deletes before the cursor, s/S replace and
    // enter insert.
    OracleCase {
        name: "delete char before cursor",
        initial_text: "abcdef\n",
        keys: "3lX",
    },
    OracleCase {
        name: "counted delete before cursor",
        initial_text: "abcdef\n",
        keys: "4l2X",
    },
    OracleCase {
        name: "substitute char",
        initial_text: "abc\n",
        keys: "sX<Esc>",
    },
    OracleCase {
        name: "counted substitute chars",
        initial_text: "abcdef\n",
        keys: "3sXY<Esc>",
    },
    OracleCase {
        name: "substitute line",
        initial_text: "  indented\nnext\n",
        keys: "Sfoo<Esc>",
    },
    OracleCase {
        name: "counted substitute lines",
        initial_text: "one\ntwo\nthree\nfour\n",
        keys: "2Sx<Esc>",
    },
    // :h w special case: dw on the buffer's last word deletes through the
    // end of that word; landing on a real final word stays exclusive.
    OracleCase {
        name: "delete word on last word of buffer",
        initial_text: "one two three\n",
        keys: "wwdw",
    },
    OracleCase {
        name: "delete word landing on short final word",
        initial_text: "one x\n",
        keys: "dw",
    },
    // Dot repeat: `.` replays the last change at the current cursor
    // position, with a typed count replacing the change's own count.
    OracleCase {
        name: "repeat char delete",
        initial_text: "abcdef\n",
        keys: "xl.",
    },
    OracleCase {
        name: "repeat delete word",
        initial_text: "one two three\n",
        keys: "dww.",
    },
    OracleCase {
        name: "repeat insert",
        initial_text: "alpha\nbeta\n",
        keys: "ihi <Esc>j0.",
    },
    OracleCase {
        name: "repeat change word",
        initial_text: "one two\nthree four\n",
        keys: "cwX<Esc>j0.",
    },
    OracleCase {
        name: "repeat with count override",
        initial_text: "abcdef\n",
        keys: "x3.",
    },
    OracleCase {
        name: "repeat keeps original count",
        initial_text: "abcdefgh\n",
        keys: "2x.",
    },
    OracleCase {
        name: "repeat remembers overridden count",
        initial_text: "abcdefghij\n",
        keys: "x3..",
    },
    OracleCase {
        name: "repeat open line below",
        initial_text: "alpha\n",
        keys: "oX<Esc>.",
    },
    OracleCase {
        name: "repeat after undo",
        initial_text: "ab\n",
        keys: "xu.",
    },
    OracleCase {
        name: "repeat paste",
        initial_text: "abc\n",
        keys: "ylp.",
    },
    OracleCase {
        name: "repeat toggle case",
        initial_text: "hello\n",
        keys: "~.",
    },
    OracleCase {
        name: "repeat replace char",
        initial_text: "abcd\n",
        keys: "rXl.",
    },
    OracleCase {
        name: "repeat join",
        initial_text: "a\nb\nc\nd\n",
        keys: "J.",
    },
    OracleCase {
        name: "repeat substitute",
        initial_text: "abc def\n",
        keys: "sX<Esc>w.",
    },
    OracleCase {
        name: "undo reverts whole repeated insert",
        initial_text: "alpha\n",
        keys: "ohi<Esc>.u",
    },
    OracleCase {
        name: "motion between change and repeat is not recorded",
        initial_text: "abc def\n",
        keys: "xww.",
    },
    // gi returns to where insert mode last stopped and resumes inserting.
    OracleCase {
        name: "go to last insert position and insert",
        initial_text: "alpha\nbeta\n",
        keys: "jA!<Esc>gggix<Esc>",
    },
    // Macros: record with q, replay with @, repeat with @@ and counts.
    OracleCase {
        name: "record and play macro",
        initial_text: "one\ntwo\nthree\n",
        keys: "qaxqj@a",
    },
    OracleCase {
        name: "replay last macro",
        initial_text: "one\ntwo\nthree\n",
        keys: "qaxqj@aj@@",
    },
    OracleCase {
        name: "counted macro play",
        initial_text: "abcdefgh\n",
        keys: "qaxq3@a",
    },
    // Registers, observed by pasting back.
    OracleCase {
        name: "named register yank and paste",
        initial_text: "alpha\nbeta\n",
        keys: "\"ayyj\"ap",
    },
    OracleCase {
        name: "named register append",
        initial_text: "one\ntwo\n",
        keys: "\"ayyj\"Ayygg\"ap",
    },
    OracleCase {
        name: "delete into named register",
        initial_text: "one two\n",
        keys: "\"adw\"ap",
    },
    OracleCase {
        name: "black hole delete keeps unnamed register",
        initial_text: "abc\ndef\n",
        keys: "yy\"_ddp",
    },
    OracleCase {
        name: "register zero keeps last yank after delete",
        initial_text: "abc\ndef\n",
        keys: "yydd\"0p",
    },
    OracleCase {
        name: "last inserted text register",
        initial_text: "x\n",
        keys: "ihello<Esc>\".p",
    },
    // Insert-mode editing keys.
    OracleCase {
        name: "insert backspace deletes typed chars",
        initial_text: "z\n",
        keys: "ihello<BS><BS><Esc>",
    },
    OracleCase {
        name: "insert backspace joins lines at line start",
        initial_text: "foo\nbar\n",
        keys: "ji<BS><Esc>",
    },
    OracleCase {
        name: "insert ctrl-w deletes word before cursor",
        initial_text: "z\n",
        keys: "ifoo bar<C-w>baz<Esc>",
    },
    OracleCase {
        name: "insert ctrl-a repeats last inserted text",
        initial_text: "x\n",
        keys: "ihey<Esc>A<C-a><Esc>",
    },
    OracleCase {
        name: "insert ctrl-r pastes named register",
        initial_text: "hello\n",
        keys: "\"ayiwA <C-r>a<Esc>",
    },
    OracleCase {
        name: "ctrl-bracket exits insert like escape",
        initial_text: "z\n",
        keys: "iab<C-[>",
    },
    // Visual mode basics.
    OracleCase {
        name: "visual charwise delete",
        initial_text: "abcdef\n",
        keys: "vlld",
    },
    OracleCase {
        name: "visual linewise delete",
        initial_text: "one\ntwo\nthree\n",
        keys: "Vjd",
    },
    OracleCase {
        name: "visual block delete",
        initial_text: "abc\nabd\n",
        keys: "l<C-v>jd",
    },
    OracleCase {
        name: "escape cancels visual selection",
        initial_text: "abc\n",
        keys: "vll<Esc>x",
    },
    OracleCase {
        name: "reselect last visual selection",
        initial_text: "alpha\nbeta\n",
        keys: "vl<Esc>jgvd",
    },
    // Plain joins (the last-line no-op pins above cover the boundary).
    OracleCase {
        name: "join lines with space",
        initial_text: "foo\nbar\nbaz\n",
        keys: "J",
    },
    OracleCase {
        name: "counted join",
        initial_text: "a\nb\nc\nd\n",
        keys: "3J",
    },
    // A counted join is one change: a single undo restores every line.
    OracleCase {
        name: "counted join then undo",
        initial_text: "one\ntwo\nthree\nfour\n",
        keys: "3Ju",
    },
    OracleCase {
        name: "counted join without space then undo",
        initial_text: "one\ntwo\nthree\nfour\n",
        keys: "3gJu",
    },
    // [<Space> and ]<Space> (Neovim defaults) add blank lines around the
    // cursor line. Above pushes the cursor down with its text, keeping the
    // column; below leaves it alone. Count, dot repeat, and undo apply.
    OracleCase {
        name: "blank line below",
        initial_text: "one\ntwo\nthree\n",
        keys: "j]<Space>",
    },
    OracleCase {
        name: "blank line above moves with the text",
        initial_text: "one\ntwo\nthree\n",
        keys: "jll[<Space>",
    },
    OracleCase {
        name: "counted blank lines below",
        initial_text: "one\ntwo\nthree\n",
        keys: "j2]<Space>",
    },
    OracleCase {
        name: "counted blank lines above",
        initial_text: "one\ntwo\nthree\n",
        keys: "j2[<Space>",
    },
    OracleCase {
        name: "blank line below on last line",
        initial_text: "one\ntwo\n",
        keys: "G]<Space>",
    },
    OracleCase {
        name: "blank line below on last line without trailing newline",
        initial_text: "one\ntwo",
        keys: "G]<Space>",
    },
    OracleCase {
        name: "blank line above on first line",
        initial_text: "one\ntwo\n",
        keys: "[<Space>",
    },
    OracleCase {
        name: "dot repeats counted blank lines below",
        initial_text: "one\ntwo\n",
        keys: "2]<Space>.",
    },
    OracleCase {
        name: "undo removes counted blank lines above",
        initial_text: "one\ntwo\nthree\n",
        keys: "jll2[<Space>u",
    },
    OracleCase {
        name: "join without added space",
        initial_text: "foo \nbar\n",
        keys: "gJ",
    },
    // Linewise gp/gP land the cursor after the pasted text (the charwise
    // forms are pinned by the "characterwise paste ... and move" cases).
    OracleCase {
        name: "linewise paste after and move",
        initial_text: "one\ntwo\n",
        keys: "yygp",
    },
    OracleCase {
        name: "linewise paste before and move",
        initial_text: "one\ntwo\n",
        keys: "yygP",
    },
    // Issue #272: cw on a non-blank must not include the trailing
    // whitespace the w motion covers (:h cw special case).
    OracleCase {
        name: "change word excludes trailing spaces",
        initial_text: "abc def  # text\n",
        keys: "llllcwxyz<Esc>",
    },
    // On the last char of a word, cw changes just that char (unlike ce,
    // which would jump to the next word's end).
    OracleCase {
        name: "change word at last char of word",
        initial_text: "ab cd ef\n",
        keys: "lcwX<Esc>",
    },
    // With a count, the stand-still at word end consumes the first count.
    OracleCase {
        name: "counted change word from word end",
        initial_text: "ab cd ef\n",
        keys: "lc2wX<Esc>",
    },
    OracleCase {
        name: "counted change word excludes trailing spaces",
        initial_text: "abc def ghi jkl\n",
        keys: "c2wX<Esc>",
    },
    // On whitespace the special case does not apply; cw keeps the
    // exclusive w range and changes the blanks up to the next word.
    OracleCase {
        name: "change word on whitespace",
        initial_text: "ab   cd\n",
        keys: "llcwX<Esc>",
    },
    OracleCase {
        name: "change big word excludes trailing spaces",
        initial_text: "a.b c.d  e\n",
        keys: "cWX<Esc>",
    },
    OracleCase {
        name: "change word stops at punctuation boundary",
        initial_text: "abc# def\n",
        keys: "cwX<Esc>",
    },
    // Insert-mode <C-v> takes the next key literally (issue #281's repro
    // inserted "vy" instead of the 0x19 control byte).
    OracleCase {
        name: "ctrl-v inserts literal control char",
        initial_text: "\n",
        keys: "i<C-v><C-y><Esc>",
    },
    OracleCase {
        name: "ctrl-v inserts literal escape and insert continues",
        initial_text: "\n",
        keys: "iA<C-v><Esc>B<Esc>",
    },
    OracleCase {
        name: "ctrl-v inserts literal tab",
        initial_text: "\n",
        keys: "iA<C-v><Tab>B<Esc>",
    },
    // The literal path must bypass auto-pairs: Vim inserts a lone paren.
    OracleCase {
        name: "ctrl-v open paren inserts without auto pair",
        initial_text: "\n",
        keys: "i<C-v>(<Esc>",
    },
    OracleCase {
        name: "ctrl-q aliases ctrl-v literal insert",
        initial_text: "\n",
        keys: "i<C-v><C-y>x<C-q><C-y><Esc>",
    },
    // The literal must belong to the surrounding insert session for undo,
    // counts, and dot repeat. A <C-v><CR> variant is blocked on ropey
    // treating a bare CR as a line break.
    OracleCase {
        name: "undo removes literal insert with its session",
        initial_text: "\n",
        keys: "i<C-v><C-y><Esc>u",
    },
    OracleCase {
        name: "counted literal insert repeats the control char",
        initial_text: "\n",
        keys: "3i<C-v><C-y><Esc>",
    },
    OracleCase {
        name: "dot repeat replays literal insert",
        initial_text: "\n",
        keys: "i<C-v><C-y><Esc>.",
    },
    // Insert <C-y>/<C-e> copy the character in the cursor's screen column
    // from the line above/below, one per press. Texts are asymmetric so a
    // copy from the wrong line or column shows up in the snapshot.
    OracleCase {
        name: "insert ctrl-y copies char from line above",
        initial_text: "abc\nxyz\n",
        keys: "jli<C-y><Esc>",
    },
    OracleCase {
        name: "insert ctrl-e copies char from line below",
        initial_text: "abc\nxyz\n",
        keys: "li<C-e><Esc>",
    },
    // Each copy advances the cursor, so presses walk along the other line
    // and stop inserting once it runs out.
    OracleCase {
        name: "repeated ctrl-e walks along the line below",
        initial_text: "ab\nwxyz\n",
        keys: "A<C-e><C-e><C-e><Esc>",
    },
    // No line to copy from, or a line that ends before the column: Vim
    // beeps and stays in insert mode.
    OracleCase {
        name: "ctrl-y on the first line inserts nothing",
        initial_text: "abc\n",
        keys: "li<C-y>Q<Esc>",
    },
    OracleCase {
        name: "ctrl-e on the last line inserts nothing",
        initial_text: "abc\nxyz\n",
        keys: "jli<C-e>Q<Esc>",
    },
    OracleCase {
        name: "ctrl-e past the end of a shorter line inserts nothing",
        initial_text: "abcdef\nxy\n",
        keys: "4li<C-e>Q<Esc>",
    },
    // Columns are screen columns: a wide character counts for two, and a
    // character that spans the cursor column is copied whole.
    OracleCase {
        name: "ctrl-y matches wide characters by screen column",
        initial_text: "日本\nabcd\n",
        keys: "jlli<C-y><Esc>",
    },
    OracleCase {
        name: "ctrl-e from a wide character copies the char in its column",
        initial_text: "日本\nabcd\n",
        keys: "li<C-e><Esc>",
    },
    OracleCase {
        name: "ctrl-e inside a wide character copies the spanning char",
        initial_text: "abcd\n日本\n",
        keys: "3li<C-e><Esc>",
    },
    // Copied like a literal: no auto pair for a paren.
    OracleCase {
        name: "ctrl-e copies an open paren without auto pairing",
        initial_text: "x\n(\n",
        keys: "i<C-e><Esc>",
    },
    OracleCase {
        name: "undo removes ctrl-y copies with the session",
        initial_text: "abc\nxyz\n",
        keys: "ji<C-y><C-y><Esc>u",
    },
    OracleCase {
        name: "counted insert repeats the copied character",
        initial_text: "ab\ncd\n",
        keys: "3i<C-e><Esc>",
    },
    // Vim's redo buffer holds the copied character, not the key, so `.`
    // re-inserts the same text instead of copying from the new neighbour.
    OracleCase {
        name: "dot repeat re-inserts the copied character not the key",
        initial_text: "ab\ncd\nef\n",
        keys: "i<C-e><Esc>j.",
    },
    // ]p and [p paste with the indent adjusted to the current line: the
    // first non-empty pasted line takes the current line's indent and the
    // rest keep their indent relative to it. Only "]p" pastes below; [p,
    // [P and ]P all paste above. Indents stay under 8 columns so Neovim's
    // noexpandtab never turns them into tabs.
    OracleCase {
        name: "bracket p pastes below with the current indent",
        initial_text: "  a\nb\n    c\n",
        keys: "jyyj]p",
    },
    OracleCase {
        name: "bracket open p pastes above with the current indent",
        initial_text: "  a\nb\n    c\n",
        keys: "jyyj[p",
    },
    OracleCase {
        name: "bracket open P pastes above with the current indent",
        initial_text: "  a\nb\n    c\n",
        keys: "jyyj[P",
    },
    OracleCase {
        name: "bracket close P pastes above with the current indent",
        initial_text: "  a\nb\n    c\n",
        keys: "jyyj]P",
    },
    OracleCase {
        name: "bracket p keeps relative indent and clamps at zero",
        initial_text: "    x\n      y\n\n  z\nq\n",
        keys: "4yyG]p",
    },
    OracleCase {
        name: "bracket p reindents whitespace-only lines",
        initial_text: "  a\n   \n  b\n    q\n",
        keys: "3yyG]p",
    },
    OracleCase {
        name: "bracket p onto an empty line removes the indent",
        initial_text: "    x\n  y\n\n",
        keys: "yyG]p",
    },
    OracleCase {
        name: "bracket p with a charwise register pastes like p",
        initial_text: "ab\n    cd\n",
        keys: "ylj]p",
    },
    // Neovim's nostartofline default: a counted G keeps column 0, so the
    // charwise put lands after the first space, not after the `c`.
    OracleCase {
        name: "bracket p charwise keeps the column counted G lands on",
        initial_text: "  a\nb\n    c\n",
        keys: "2Gyl3G]p",
    },
    // A multi-line charwise register keeps its first line merged into the
    // current line untouched; the lines after it get the indent fix, with
    // the first of them taking the current line's indent. Not covered: a
    // charwise register whose LAST line is empty or whitespace-only. Vim
    // re-indents that merged line including the tail's own whitespace;
    // Nevi leaves the tail alone. Rare enough to leave undone.
    OracleCase {
        name: "bracket p with a multiline charwise register",
        initial_text: "ab\ncd\n    q\n",
        keys: "vjyG]p",
    },
    // Plain multi-line charwise puts leave the cursor on the first pasted
    // character. `p` used to land one column right of it (single-line math
    // applied to multi-line text); `P` was already right.
    OracleCase {
        name: "multiline charwise paste after leaves cursor at its start",
        initial_text: "ab\ncd\n    q\n",
        keys: "vjyGp",
    },
    OracleCase {
        name: "multiline charwise paste before leaves cursor at its start",
        initial_text: "ab\ncd\n    q\n",
        keys: "vjyGP",
    },
    OracleCase {
        name: "counted bracket p indents every copy",
        initial_text: "  a\nb\n    c\n",
        keys: "jyyj3]p",
    },
    OracleCase {
        name: "bracket p from a named register",
        initial_text: "  a\nb\n    c\n",
        keys: "j\"ayyj\"a]p",
    },
    OracleCase {
        name: "undo bracket p",
        initial_text: "  a\nb\n    c\n",
        keys: "jyyj]pu",
    },
    OracleCase {
        name: "dot repeat bracket p",
        initial_text: "a\n  b\n    c\n",
        keys: "yyj]pj.",
    },
];
