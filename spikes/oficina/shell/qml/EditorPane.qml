// The editor pane: one TOML file as a form and as text, two views of one document (the spike's
// point 2). The form is the document's fields; the text view is a native text editor, the plan's
// fallback where Chromium cannot draw.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Oficina

Rectangle {
    id: pane
    color: "#1B1B1E"

    function open(path) {
        doc.open(path);
    }

    // True while the text view hands its text to the document, so the document's answer is not
    // written back over what is being typed.
    property bool fromText: false

    TomlDocument {
        id: doc
        objectName: "document"
    }

    Connections {
        target: doc
        function onTextChanged() {
            if (!pane.fromText) {
                textView.syncing = true;
                textView.text = doc.text;
                textView.syncing = false;
            }
        }
    }

    RowLayout {
        id: bar
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 4 }
        spacing: 8
        Button {
            objectName: "hello"
            property int clicks: 0
            text: "clicked " + clicks
            onClicked: clicks += 1
        }
        Text {
            objectName: "fileName"
            text: doc.path
            textFormat: Text.PlainText
            color: "#E8E8EC"
            elide: Text.ElideLeft
            Layout.fillWidth: true
        }
        Text {
            objectName: "stamp"
            text: doc.stamp
            textFormat: Text.PlainText
            color: "#9A9AA3"
        }
        Button {
            objectName: "save"
            text: qsTr("Save")
            enabled: doc.modified && !doc.stale
            onClicked: doc.save()
        }
    }

    Text {
        id: errorLine
        objectName: "editorError"
        anchors { left: parent.left; right: parent.right; top: bar.bottom; margins: 4 }
        text: doc.error
        textFormat: Text.PlainText
        color: "#FF8F8F"
        wrapMode: Text.Wrap
    }

    SplitView {
        anchors { left: parent.left; right: parent.right; top: errorLine.bottom; bottom: parent.bottom }
        orientation: Qt.Horizontal

        ListView {
            id: form
            objectName: "form"
            SplitView.preferredWidth: pane.width / 2
            clip: true
            model: doc
            // While the text does not parse, the form shows the document's last state, untouched.
            enabled: !doc.stale
            opacity: doc.stale ? 0.5 : 1
            delegate: RowLayout {
                id: fieldRow
                required property int index
                required property string path
                required property string kind
                required property string value
                required property string comment
                required property bool stamp
                width: form.width
                spacing: 6
                Text {
                    text: fieldRow.path
                    textFormat: Text.PlainText
                    color: "#7F8088"
                    font.family: "monospace"
                    Layout.preferredWidth: form.width * 0.4
                    elide: Text.ElideMiddle
                }
                TextField {
                    objectName: "field" + fieldRow.index
                    text: fieldRow.value
                    readOnly: fieldRow.stamp
                    font.family: "monospace"
                    Layout.fillWidth: true
                    onEditingFinished: {
                        if (text !== fieldRow.value)
                            doc.setField(fieldRow.index, text);
                    }
                }
                Text {
                    text: fieldRow.comment
                    textFormat: Text.PlainText
                    color: "#8A8A92"
                    Layout.preferredWidth: form.width * 0.2
                    elide: Text.ElideRight
                }
            }
        }

        ScrollView {
            // What a person clicks to type in the text: its visible part (the text view itself grows
            // with its text, its centre below the view for a long file).
            objectName: "textScroll"
            SplitView.fillWidth: true
            TextArea {
                id: textView
                objectName: "textView"
                property bool syncing: false
                textFormat: TextEdit.PlainText
                font.family: "monospace"
                wrapMode: TextEdit.NoWrap
                color: "#D4D4DA"
                // Where the text that does not parse fails, at its character.
                Rectangle {
                    objectName: "errorMarker"
                    readonly property int line: doc.errorLine
                    readonly property int column: doc.errorColumn
                    readonly property rect at: textView.positionToRectangle(Math.min(doc.errorOffset, textView.length))
                    visible: doc.stale
                    x: at.x
                    y: at.y
                    width: Math.max(at.width, 8)
                    height: at.height
                    color: "transparent"
                    border.color: "#FF6B6B"
                }
                onTextChanged: {
                    if (!syncing) {
                        pane.fromText = true;
                        doc.editText(text);
                        pane.fromText = false;
                    }
                }
            }
        }
    }
}
