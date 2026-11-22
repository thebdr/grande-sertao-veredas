import QtQuick
import QtQuick.Controls
import com.kdab.dockwidgets 2.0 as KDDW

ApplicationWindow {
    id: window
    objectName: "oficina"
    visible: true
    width: 1280
    height: 800
    title: "Oficina"
    color: "#141416"

    KDDW.DockingArea {
        id: area
        objectName: "area"
        anchors.fill: parent
        uniqueName: "workbench"

        // The workbench's named layouts (the spike's point 7): saved to a file and restored from
        // it by KDDockWidgets' own saver. `layoutState` describes the panes as they stand: each
        // dock's name, open, floating, the window its pane is drawn in and its place there;
        // `layoutAnswer` what the last save or restore said, numbered.
        property string layoutState: ""
        property string layoutAnswer: ""
        property int layoutCalls: 0

        KDDW.LayoutSaver {
            id: saver
        }

        function saveLayout(path) {
            layoutCalls += 1;
            layoutAnswer = layoutCalls + (saver.saveToFile(path) ? " saved" : " not saved");
        }

        function restoreLayout(path) {
            layoutCalls += 1;
            layoutAnswer = layoutCalls + (saver.restoreFromFile(path) ? " restored" : " not restored");
        }

        // `tag` heads the description, so a test tells this one from one before it.
        function describeLayout(tag) {
            const docks = [[projectDock, projectPane], [editorDock, editor], [tableDock, tablePane], [webDock, webPane], [codeDock, codeEditor]];
            layoutState = tag + " " + JSON.stringify(docks.map(([dock, pane]) => {
                const at = pane.mapToItem(null, 0, 0);
                const drawn = pane.Window.window === null ? "none" : (pane.Window.window === window ? "main" : "own");
                // A closed dock reads as floating (it has no place in the window): said only of an open one.
                return [dock.uniqueName, dock.isOpen, dock.isOpen ? dock.isFloating : null, dock.isOpen ? [drawn, Math.round(at.x), Math.round(at.y), Math.round(pane.width), Math.round(pane.height)] : null];
            }));
        }

        // What a person does with a dock's title bar: close it, or float it out of the window.
        function closeDock(name) {
            dockNamed(name).close();
        }

        function floatDock(name) {
            dockNamed(name).isFloating = true;
        }

        function dockNamed(name) {
            return [projectDock, editorDock, tableDock, webDock, codeDock].find((d) => d.uniqueName === name);
        }

        KDDW.DockWidget {
            id: projectDock
            uniqueName: "project"
            title: qsTr("Project")
            ProjectPane {
                id: projectPane
                anchors.fill: parent
                onOpenFile: (path) => path.endsWith(".rs") ? codeEditor.open(path) : editor.open(path)
            }
        }

        KDDW.DockWidget {
            id: editorDock
            uniqueName: "editor"
            title: qsTr("Editor")
            EditorPane {
                id: editor
                anchors.fill: parent
            }
        }

        KDDW.DockWidget {
            id: tableDock
            uniqueName: "table"
            title: qsTr("Table")
            TablePane {
                id: tablePane
                anchors.fill: parent
            }
        }

        KDDW.DockWidget {
            id: webDock
            uniqueName: "web"
            title: qsTr("Page")
            WebPane {
                id: webPane
                anchors.fill: parent
            }
        }

        KDDW.DockWidget {
            id: codeDock
            uniqueName: "code"
            title: qsTr("Code")
            CodePane {
                id: codeEditor
                anchors.fill: parent
            }
        }

        Component.onCompleted: {
            addDockWidget(projectDock, KDDW.KDDockWidgets.Location_OnLeft, null, Qt.size(320, 800));
            addDockWidget(editorDock, KDDW.KDDockWidgets.Location_OnRight);
            addDockWidget(tableDock, KDDW.KDDockWidgets.Location_OnBottom, editorDock);
            addDockWidget(webDock, KDDW.KDDockWidgets.Location_OnRight, tableDock);
            addDockWidget(codeDock, KDDW.KDDockWidgets.Location_OnRight, editorDock);
        }
    }
}
