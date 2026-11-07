import QtQuick
import QtQuick.Window
import QtQuick.Controls as QQC
import Qt.labs.platform as Platform
import com.ferric.app 1.0

// IDM's "Download File Info" dialog: shown for every browser grab
// and every Add URL, with Start Download / Download Later / Cancel. The download is held by the
// daemon until one of them is pressed; closing the window is Cancel.
Window {
    id: win

    property string downloadId: ""
    property var requestBus
    property var folders: []        // [{ name, dir }] from the daemon: General, defaults, user categories
    property var rec: null
    property bool handled: false
    // IDM's "Show queue selection panel on pressing Download Later"
    property var queues: []
    property bool askQueue: true
    property string laterQueue: ""
    property string fileName: ""
    property string category: "General"
    property string dirPath: ""
    property string errorText: ""
    // Once the user changes the name, folder or category, a late probe result must not overwrite it.
    property bool touched: false

    signal finished(string id)

    width: 590
    height: 268
    minimumWidth: width
    maximumWidth: width
    minimumHeight: height
    maximumHeight: height
    flags: Qt.Dialog | Qt.WindowTitleHint | Qt.WindowCloseButtonHint | Qt.WindowStaysOnTopHint
    color: Theme.bg
    title: qsTr("Download File Info")

    readonly property string sep: Qt.platform.os === "windows" ? "\\" : "/"
    // A playlist's own length is not the size of the stream it describes.
    readonly property bool isPlaylist: rec ? /\.(m3u8|mpd)(\?|#|$)/i.test(rec.url || "") : false
    readonly property real total: !isPlaylist && rec && rec.total_size ? rec.total_size : 0
    readonly property string ext: {
        var m = /\.([A-Za-z0-9]{1,5})$/.exec(win.fileName);
        return m ? m[1].toUpperCase() : "";
    }

    function splitPath(p) {
        var i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
        return i < 0 ? { dir: "", name: p } : { dir: p.substring(0, i), name: p.substring(i + 1) };
    }
    function joinPath(dir, name) {
        if (dir === "") return name;
        var last = dir.charAt(dir.length - 1);
        return dir + (last === "\\" || last === "/" ? "" : win.sep) + name;
    }
    function folderFor(cat) {
        for (var i = 0; i < win.folders.length; i++)
            if (win.folders[i].name === cat) return win.folders[i].dir;
        return "";
    }
    function fmtSize(n) {
        // e.g. "351 Bytes", "1.08 MB"
        if (!n || n <= 0) return "";
        if (n < 1024) return n + " Bytes";
        if (n < 1048576) return (n / 1024).toFixed(2) + " KB";
        if (n < 1073741824) return (n / 1048576).toFixed(2) + " MB";
        return (n / 1073741824).toFixed(2) + " GB";
    }
    function syncSaveField() { saveField.text = win.joinPath(win.dirPath, win.fileName); }

    function load() {
        if (!downloadId || !requestBus) return;
        requestBus("get_category_folders", {}, function(raw) {
            try { var f = JSON.parse(raw); if (f instanceof Array) win.folders = f; } catch (e) {}
            win.refreshProbe();
        });
    }

    // Called on load and again when the daemon's probe of the address finishes (IDM fills the dialog in the same way).
    function refreshProbe() {
        requestBus("get_download", { id: win.downloadId }, function(raw2) {
            var d;
            try { d = JSON.parse(raw2); } catch (e) { return; }
            if (!d || d.error) { win.errorText = d && d.error ? d.error : qsTr("Download not found"); return; }
            win.rec = d;
            if (win.touched) return;
            var p = win.splitPath(d.save_path || "");
            win.fileName = d.filename || p.name;
            win.category = d.category && d.category !== "Other" ? d.category : "General";
            win.dirPath = p.dir !== "" ? p.dir : win.folderFor(win.category);
            var idx = 0;
            for (var i = 0; i < win.folders.length; i++) if (win.folders[i].name === win.category) idx = i;
            catCombo.currentIndex = idx;
            win.syncSaveField();
        });
    }

    function finish() { win.finished(win.downloadId); win.close(); }

    function confirm(start) {
        if (win.handled || !win.rec) return;
        var parts = win.splitPath(saveField.text.trim());
        if (parts.name === "" || parts.dir === "") { win.errorText = qsTr("Enter a full path and file name"); return; }
        win.handled = true;
        var args = {
            id: win.downloadId,
            filename: parts.name,
            save_dir: parts.dir,
            description: descField.text.trim(),
            category: win.category,
            start: start
        };
        var send = function() {
            win.requestBus("confirm_download", args, function(err) {
                if (err) { win.handled = false; win.errorText = err; return; }
                if (!start && win.laterQueue !== "")
                    win.requestBus("move_download_to_queue", { id: win.downloadId, queue_id: win.laterQueue }, function() { win.finish(); });
                else
                    win.finish();
            });
        };
        if (rememberBox.checked)
            win.requestBus("set_category_folder", { name: win.category, dir: parts.dir }, function() { send(); });
        else
            send();
    }

    function cancel() {
        if (win.handled) return;
        win.handled = true;
        win.requestBus("delete_download", { id: win.downloadId, with_file: false }, function() { win.finish(); });
    }

    onClosing: function(close) {
        // The window's X is Cancel, exactly like IDM.
        if (!win.handled) { win.handled = true; win.requestBus("delete_download", { id: win.downloadId, with_file: false }, function() {}); win.finished(win.downloadId); }
    }
    Component.onCompleted: load()

    Platform.FileDialog {
        id: browseDialog
        title: qsTr("Save as")
        fileMode: Platform.FileDialog.SaveFile
        onAccepted: {
            var p = file.toString().replace(/^file:\/\/\//, "").replace(/^file:\/\//, "");
            if (p.length >= 3 && p.charAt(0) === "/" && p.charAt(2) === ":") p = p.substring(1);
            p = decodeURIComponent(p);
            if (win.sep === "\\") p = p.replace(/\//g, "\\");
            var s = win.splitPath(p);
            win.touched = true;
            win.dirPath = s.dir; win.fileName = s.name; win.syncSaveField();
        }
    }

    component Field: Rectangle {
        property alias text: edit.text
        property alias editor: edit
        property bool readOnly: false
        height: 26; radius: 2
        color: readOnly ? Theme.surfaceAlt : Theme.surface
        border.color: edit.activeFocus ? Theme.accent : Theme.border
        TextInput {
            id: edit
            anchors { fill: parent; leftMargin: Theme.sp1; rightMargin: Theme.sp1 }
            verticalAlignment: TextInput.AlignVCenter
            color: Theme.text; font.pixelSize: Theme.fontSizeNormal
            readOnly: parent.readOnly
            selectByMouse: true
            clip: true
        }
    }

    Column {
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: Theme.sp3 }
        spacing: Theme.sp2

        // URL (read-only, like IDM)
        Row {
            spacing: Theme.sp2; height: 26
            Text { text: qsTr("URL"); width: 78; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; anchors.verticalCenter: parent.verticalCenter; horizontalAlignment: Text.AlignRight }
            Field { id: urlField; width: win.width - 28 - 86; readOnly: true; text: win.rec ? (win.rec.final_url || win.rec.url) : "" }
        }

        // Category + "+"
        Row {
            spacing: Theme.sp2; height: 26
            Text { text: qsTr("Category"); width: 78; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; anchors.verticalCenter: parent.verticalCenter; horizontalAlignment: Text.AlignRight }
            QQC.ComboBox {
                id: catCombo
                width: 200; height: 26
                model: win.folders
                textRole: "name"
                font.pixelSize: Theme.fontSizeNormal
                onActivated: function(i) {
                    if (i < 0 || i >= win.folders.length) return;
                    win.touched = true;
                    win.category = win.folders[i].name;
                    win.dirPath = win.folders[i].dir;
                    win.syncSaveField();
                }
                background: Rectangle { radius: 2; color: Theme.surface; border.color: catCombo.activeFocus ? Theme.accent : Theme.border }
                contentItem: Text { leftPadding: Theme.sp1; text: catCombo.displayText; color: Theme.text; font.pixelSize: Theme.fontSizeNormal; verticalAlignment: Text.AlignVCenter; elide: Text.ElideRight }
            }
        }

        // Save As + "..."
        Row {
            spacing: Theme.sp2; height: 26
            Text { text: qsTr("Save As"); width: 78; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; anchors.verticalCenter: parent.verticalCenter; horizontalAlignment: Text.AlignRight }
            Field {
                id: saveField
                width: win.width - 28 - 86 - 34 - 130
                editor.onTextEdited: win.touched = true
                editor.onEditingFinished: { var s = win.splitPath(text); if (s.dir !== "") win.dirPath = s.dir; if (s.name !== "") win.fileName = s.name; }
            }
            Rectangle {
                width: 26; height: 26; radius: 2; color: Theme.surfaceAlt; border.color: Theme.border
                Text { anchors.centerIn: parent; text: "…"; color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
                MouseArea { anchors.fill: parent; onClicked: browseDialog.open() }
            }
        }

        // Remember path for this category
        Row {
            spacing: Theme.sp2
            Item { width: 78; height: 1 }
            Column {
                spacing: Theme.sp1
                Row {
                    spacing: Theme.sp1
                    Rectangle {
                        id: rememberBox
                        property bool checked: false
                        width: 16; height: 16; radius: 3
                        color: checked ? Theme.accent : Theme.bg; border.color: Theme.border
                        Text { anchors.centerIn: parent; text: parent.checked ? "✓" : ""; color: "#fff"; font.pixelSize: Theme.fontSizeSmall }
                        MouseArea { anchors.fill: parent; onClicked: rememberBox.checked = !rememberBox.checked }
                    }
                    Text { text: qsTr("Remember this path for \"%1\" category").arg(win.category); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                }
                Field {
                    width: win.width - 28 - 86 - 130   // leaves room for the Preview button
                    readOnly: !rememberBox.checked
                    text: win.dirPath
                    editor.onEditingFinished: if (rememberBox.checked) { win.dirPath = text; win.syncSaveField(); }
                }
            }
        }

        // Description
        Row {
            spacing: Theme.sp2; height: 26
            Text { text: qsTr("Description"); width: 78; color: Theme.textDim; font.pixelSize: Theme.fontSizeNormal; anchors.verticalCenter: parent.verticalCenter; horizontalAlignment: Text.AlignRight }
            Field { id: descField; width: win.width - 28 - 86 }
        }
    }

    // file type badge and size, top right like IDM (beside the Category and Save As rows)
    Component { id: previewComp; ZipPreviewWindow { requestBus: win.requestBus } }

    Item {
        anchors { right: parent.right; rightMargin: Theme.sp3; top: parent.top; topMargin: Theme.sp5 }
        width: 120; height: 70
        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 36; height: 44; radius: 3
            color: Theme.surface; border.color: Theme.accent; border.width: 2
            Text { anchors.centerIn: parent; text: win.ext; color: Theme.accent; font.pixelSize: win.ext.length > 3 ? 9 : 11; font.bold: true }
        }
        Text {
            anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom }
            text: win.fmtSize(win.total)
            color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall
        }
    }

    // IDM's Preview: shown for zip archives once the size is known
    Rectangle {
        visible: win.ext === "ZIP" && win.total > 0
        anchors { right: parent.right; rightMargin: Theme.sp3; top: parent.top; topMargin: Theme.sp6 }
        width: 90; height: 24; radius: 3; color: Theme.surfaceAlt; border.color: Theme.border
        Text { anchors.centerIn: parent; text: qsTr("Preview"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
        MouseArea {
            anchors.fill: parent
            onClicked: {
                var w = previewComp.createObject(null, { downloadId: win.downloadId });
                if (w) { w.closing.connect(function() { w.destroy(500); }); w.show(); }
            }
        }
    }

    // buttons
    Row {
        anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: Theme.sp3 }
        spacing: Theme.sp3
        Rectangle {
            width: 100; height: 28; radius: 3; color: Theme.surfaceAlt; border.color: Theme.border
            Text { anchors.centerIn: parent; text: qsTr("Download Later"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { anchors.fill: parent; onClicked: {
                if (win.askQueue && win.queues.length > 1) laterQueueMenu.popup()
                else win.confirm(false)
            } }
            QQC.Menu {
                id: laterQueueMenu
                Repeater {
                    model: win.queues
                    QQC.MenuItem { text: modelData.name; onTriggered: { win.laterQueue = modelData.id; win.confirm(false) } }
                }
            }
        }
        Rectangle {
            width: 100; height: 28; radius: 3; color: Theme.accent
            Text { anchors.centerIn: parent; text: qsTr("Start Download"); color: "white"; font.pixelSize: Theme.fontSizeNormal; font.bold: true }
            MouseArea { anchors.fill: parent; onClicked: win.confirm(true) }
        }
        Rectangle {
            width: 100; height: 28; radius: 3; color: Theme.surfaceAlt; border.color: Theme.border
            Text { anchors.centerIn: parent; text: qsTr("Cancel"); color: Theme.text; font.pixelSize: Theme.fontSizeNormal }
            MouseArea { anchors.fill: parent; onClicked: win.cancel() }
        }
    }

    Text {
        anchors { left: parent.left; leftMargin: Theme.sp3; right: parent.right; rightMargin: Theme.sp3; bottom: parent.bottom; bottomMargin: Theme.sp6 }
        visible: win.errorText !== ""
        text: win.errorText
        color: Theme.danger; font.pixelSize: Theme.fontSizeSmall; elide: Text.ElideRight
    }
}
