import QtQuick
import QtQuick.Window
import com.ferric.app 1.0

// IDM-style "Download complete" window: size line, Address + saved-as fields,
// Open / Open folder / Close and a "don't show again" toggle.
Window {
    id: win

    property var info: null          // DownloadRecord JSON
    property var bus
    property var requestBus
    property bool dontShowAgain: false
    // the record's save_path is already the full file path
    readonly property string fullPath: info ? info.save_path.replace(/\\/g, "/") : ""

    width: 470
    height: 236
    minimumWidth: 470; maximumWidth: 470; minimumHeight: 236; maximumHeight: 236
    color: Theme.bg
    title: qsTr("Download complete")

    function sizeLine() {
        if (!info) return "";
        var n = info.total_size || info.downloaded || 0;
        var human = n < 1048576 ? (n / 1024).toFixed(2) + " KB" : (n / 1048576).toFixed(2) + " MB";
        return qsTr("Downloaded ") + human + " (" + n.toLocaleString(Qt.locale("en_US"), "f", 0) + " Bytes)";
    }

    onClosing: {
        if (dontShowAgain && requestBus)
            requestBus("set_setting", { key: "show_complete_dialog", value: "0" }, function() {});
    }

    Column {
        anchors { fill: parent; margins: Theme.sp3 }
        spacing: Theme.sp2

        Text { text: win.sizeLine(); color: Theme.text; font.pixelSize: Theme.fontSizeNormal; font.bold: true }

        Text { text: qsTr("Address"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        Rectangle {
            width: parent.width; height: 24; color: Theme.surface; border.color: Theme.border
            TextInput {
                anchors { fill: parent; margins: Theme.sp1 }
                readOnly: true; selectByMouse: true; clip: true
                text: win.info ? (win.info.final_url || win.info.url) : ""
                color: Theme.text; font.pixelSize: Theme.fontSizeSmall
            }
        }

        Text { text: qsTr("The file saved as"); color: Theme.textDim; font.pixelSize: Theme.fontSizeSmall }
        Rectangle {
            width: parent.width; height: 24; color: Theme.surface; border.color: Theme.border
            TextInput {
                anchors { fill: parent; margins: Theme.sp1 }
                readOnly: true; selectByMouse: true; clip: true
                text: win.fullPath
                color: Theme.text; font.pixelSize: Theme.fontSizeSmall
            }
        }

        Row {
            spacing: Theme.sp2
            Rectangle {
                width: 90; height: 26; radius: 3; color: Theme.accent
                Text { anchors.centerIn: parent; text: qsTr("Open"); color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: win.bus.open_file(win.fullPath) }
            }
            Rectangle {
                width: 110; height: 26; radius: 3; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Open folder"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: win.bus.open_folder(win.fullPath) }
            }
            Rectangle {
                width: 80; height: 26; radius: 3; color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: qsTr("Close"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: win.close() }
            }
        }

        Row {
            spacing: Theme.sp2
            Rectangle {
                width: 16; height: 16; radius: 3
                color: win.dontShowAgain ? Theme.accent : Theme.bg
                border.color: Theme.surfaceAlt
                Text { anchors.centerIn: parent; text: win.dontShowAgain ? "✓" : ""; color: "white"; font.pixelSize: Theme.fontSizeSmall }
                MouseArea { anchors.fill: parent; onClicked: win.dontShowAgain = !win.dontShowAgain }
            }
            Text { text: qsTr("Don't show this dialog again"); color: Theme.text; font.pixelSize: Theme.fontSizeSmall }
        }
    }
}
