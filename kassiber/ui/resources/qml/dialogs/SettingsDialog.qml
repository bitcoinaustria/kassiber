import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Dialog {
    id: root
    title: "Settings"
    modal: true
    width: 480
    standardButtons: Dialog.Close

    contentItem: ColumnLayout {
        spacing: theme.spacingMd

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "Phase 1 only wires the shell and persists window geometry."
            color: theme.ink
            font.family: theme.displayFont
            font.pixelSize: 20
        }

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: "A fuller Settings surface lands in Phase 4. For now the useful desktop state lives in the managed settings manifest."
            color: theme.inkMuted
            font.family: theme.bodyFont
            font.pixelSize: 13
        }

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WrapAnywhere
            text: "settings.json: " + settingsVM.settingsFile
            color: theme.inkMuted
            font.family: theme.bodyFont
            font.pixelSize: 12
        }

        Text {
            Layout.fillWidth: true
            wrapMode: Text.WrapAnywhere
            text: "env file: " + settingsVM.envFile
            color: theme.inkMuted
            font.family: theme.bodyFont
            font.pixelSize: 12
        }
    }
}
