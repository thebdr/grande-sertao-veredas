// The project pane: the tree of the folder named on the command line (the spike's point 1).
import QtQuick
import QtQuick.Controls
import QtQml.Models
import Oficina

Rectangle {
    id: pane
    color: "#161618"

    // A file picked in the tree, to open in the editor.
    signal openFile(string path)

    ProjectTree {
        id: projectTree
        objectName: "projectTree"
        Component.onCompleted: {
            const args = Qt.application.arguments;
            if (args.length > 1)
                open(args[1]);
        }
    }

    Row {
        id: bar
        height: 28
        spacing: 6
        ToolButton {
            objectName: "refreshProject"
            text: qsTr("Refresh")
            onClicked: projectTree.refreshOpen()
        }
        Text {
            objectName: "projectError"
            text: projectTree.error
            textFormat: Text.PlainText
            color: "#FF8F8F"
            anchors.verticalCenter: parent.verticalCenter
        }
    }

    TreeView {
        id: treeView
        objectName: "projectView"
        anchors { top: bar.bottom; left: parent.left; right: parent.right; bottom: parent.bottom }
        clip: true
        // A row's delegate is made for it and destroyed with it: a delegate kept for reuse keeps
        // its name, and a test reading rows by name would read a row that left.
        reuseItems: false
        model: projectTree
        selectionModel: ItemSelectionModel { model: projectTree }

        delegate: Item {
            id: node
            required property TreeView treeView
            required property bool isTreeNode
            required property bool expanded
            required property bool hasChildren
            required property int depth
            required property int row
            required property int column
            required property bool current
            required property string display
            required property string kind
            required property string detail
            required property string state
            required property bool hidden
            required property string path

            objectName: "row" + row
            // What the row says, read whole by a test.
            readonly property string label: display + (detail.length > 0 ? " | " + detail : "")
            implicitWidth: treeView.width
            implicitHeight: 22

            Rectangle {
                anchors.fill: parent
                color: node.current ? "#2B2838" : "transparent"
                border.color: node.current ? "#494360" : "transparent"
            }
            Text {
                x: node.depth * 14 + 4
                anchors.verticalCenter: parent.verticalCenter
                text: node.hasChildren ? (node.expanded ? "▾" : "▸") : ""
                textFormat: Text.PlainText
                color: "#9A9AA3"
            }
            Text {
                id: name
                objectName: "name"
                x: node.depth * 14 + 18
                anchors.verticalCenter: parent.verticalCenter
                text: node.display
                // A name is the plant's text, never markup (the review's entry 34).
                textFormat: Text.PlainText
                color: node.hidden ? "#8A8A92" : "#E8E8EC"
                font.family: "monospace"
            }
            Text {
                objectName: "detail"
                anchors { left: name.right; leftMargin: 10; verticalCenter: parent.verticalCenter }
                text: node.detail
                textFormat: Text.PlainText
                color: node.state === "refused" ? "#FFC864" : "#7F8088"
            }
            MouseArea {
                anchors.fill: parent
                onClicked: {
                    node.treeView.selectionModel.setCurrentIndex(node.treeView.index(node.row, node.column), ItemSelectionModel.NoUpdate);
                    if (node.hasChildren)
                        node.treeView.toggleExpanded(node.row);
                    else if (node.kind === "file")
                        pane.openFile(node.path);
                }
            }
        }
    }
}
