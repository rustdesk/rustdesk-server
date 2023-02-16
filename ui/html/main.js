import 'codemirror/lib/codemirror.css';
import './style.css';
import 'codemirror/mode/toml/toml.js';
import CodeMirror from 'codemirror';

const { event, fs, path, tauri } = window.__TAURI__;

class View {
    constructor() {
        Object.assign(this, {
            content: '',
            action_time: 0,
            is_auto_scroll: true,
            is_edit_mode: false,
            is_file_changed: false,
            is_form_changed: false,
            is_content_changed: false
        }, ...arguments);
        addEventListener('DOMContentLoaded', this.init.bind(this));
    }
    async init() {
        this.editor = this.renderEditor();
        this.editor.on('scroll', this.editorScroll.bind(this));
        this.editor.on('keypress', this.editorSave.bind(this));
        this.form = this.renderForm();
        this.form.addEventListener('change', this.formChange.bind(this));
        event.listen('__update__', this.appAction.bind(this));
        event.emit('__action__', '__init__');
        while (true) {
            let now = Date.now();
            try {
                await this.update();
                this.render();
            } catch (e) {
                console.error(e);
            }
            await new Promise(r => setTimeout(r, Math.max(0, 33 - (Date.now() - now))));
        }
    }
    async update() {
        if (this.is_file_changed) {
            this.is_file_changed = false;
            let now = Date.now(),
                file = await path.resolveResource(this.file);
            if (await fs.exists(file)) {
                let content = await fs.readTextFile(file);
                if (this.action_time < now) {
                    this.content = content;
                    this.is_content_changed = true;
                }
            } else {
                if (now >= this.action_time) {
                    if (this.is_edit_mode) {
                        this.content = `# https://github.com/rustdesk/rustdesk-server#env-variables
RUST_LOG=info
`;
                    }
                    this.is_content_changed = true;
                }
                console.warn(`${this.file} file is missing`);
            }
        }
    }
    async editorSave(editor, e) {
        if (e.ctrlKey && e.keyCode === 19 && this.is_edit_mode && !this.locked) {
            this.locked = true;
            try {
                let now = Date.now(),
                    content = this.editor.doc.getValue(),
                    file = await path.resolveResource(this.file);
                await fs.writeTextFile(file, content);
                event.emit('__action__', 'restart');
            } catch (e) {
                console.error(e);
            } finally {
                this.locked = false;
            }
        }
    }
    editorScroll(e) {
        let info = this.editor.getScrollInfo(),
            distance = info.height - info.top - info.clientHeight,
            is_end = distance < 1;
        if (this.is_auto_scroll !== is_end) {
            this.is_auto_scroll = is_end;
            this.is_form_changed = true;
        }
    }
    formChange(e) {
        switch (e.target.tagName.toLowerCase()) {
            case 'input':
                this.is_auto_scroll = e.target.checked;
                break;
        }
    }
    appAction(e) {
        let [action, data] = e.payload;
        switch (action) {
            case 'file':
                if (data === '.env') {
                    this.is_edit_mode = true;
                    this.file = `bin/${data}`;
                } else {
                    this.is_edit_mode = false;
                    this.file = `logs/${data}`;
                }
                this.action_time = Date.now();
                this.is_file_changed = true;
                this.is_form_changed = true;
                break;
        }
    }
    render() {
        if (this.is_form_changed) {
            this.is_form_changed = false;
            this.renderForm();
        }
        if (this.is_content_changed) {
            this.is_content_changed = false;
            this.renderEditor();
        }
        if (this.is_auto_scroll && !this.is_edit_mode) {
            this.renderScrollbar();
        }
    }
    renderForm() {
        let form = this.form || document.querySelector('form'),
            label = form.querySelectorAll('label'),
            input = form.querySelector('input');
        input.checked = this.is_auto_scroll;
        if (this.is_edit_mode) {
            label[0].style.display = 'none';
            label[1].style.display = 'block';
        } else {
            label[0].style.display = 'block';
            label[1].style.display = 'none';
        }
        return form;
    }
    renderEditor() {
        let editor = this.editor || CodeMirror.fromTextArea(document.querySelector('textarea'), {
            mode: { name: 'toml' },
            lineNumbers: true,
            autofocus: true
        });
        editor.setOption('readOnly', !this.is_edit_mode);
        editor.doc.setValue(this.content);
        editor.doc.clearHistory();
        this.content = '';
        editor.focus();
        return editor;
    }
    renderScrollbar() {
        let info = this.editor.getScrollInfo();
        this.editor.scrollTo(info.left, info.height);
    }
}

new View();