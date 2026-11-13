import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    property alias cfg_showCpu: showCpu.checked
    property alias cfg_showMemory: showMemory.checked
    property alias cfg_showGpu: showGpu.checked
    property alias cfg_showVram: showVram.checked
    property bool cfg_showVramDefault
    property var cfg_hiddenGpus: []
    property alias cfg_showDisk: showDisk.checked
    property string cfg_diskMode: "usage"
    property alias cfg_diskSensor: diskSensor.text
    property alias cfg_showNetwork: showNetwork.checked
    property alias cfg_showGraphs: showGraphs.checked
    property alias cfg_colorLabels: colorLabels.checked
    property alias cfg_valuesInside: valuesInside.checked
    property bool cfg_valuesInsideDefault
    property string cfg_labels: "short"
    property string cfg_labelsDefault
    property alias cfg_graphWidth: graphWidth.value
    property alias cfg_updateInterval: interval.value
    // Defaults the dialog's "Defaults" button resets to (kept for KCM API).
    property bool cfg_showCpuDefault; property bool cfg_showMemoryDefault; property bool cfg_showGpuDefault
    property var cfg_hiddenGpusDefault; property bool cfg_showDiskDefault; property string cfg_diskModeDefault
    property string cfg_diskSensorDefault; property bool cfg_showNetworkDefault; property bool cfg_showGraphsDefault
    property bool cfg_colorLabelsDefault; property int cfg_graphWidthDefault; property int cfg_updateIntervalDefault

    function setGpu(i, shown) {
        const h = cfg_hiddenGpus.filter(x => x !== String(i))
        if (!shown) h.push(String(i))
        cfg_hiddenGpus = h
    }

    Kirigami.FormLayout {
        QQC2.CheckBox { id: showCpu; Kirigami.FormData.label: i18n("Show:"); text: i18n("CPU") }
        QQC2.CheckBox { id: showMemory; text: i18n("Memory") }
        QQC2.CheckBox { id: showGpu; text: i18n("GPU") }
        Repeater {
            model: 4
            QQC2.CheckBox {
                visible: showGpu.checked
                text: i18n("   GPU %1", index + 1)
                checked: cfg_hiddenGpus.indexOf(String(index)) < 0
                onToggled: setGpu(index, checked)
            }
        }
        QQC2.CheckBox { id: showVram; visible: showGpu.checked; text: i18n("   GPU memory (VRAM)") }
        QQC2.CheckBox { id: showDisk; text: i18n("Disk") }
        QQC2.CheckBox { id: showNetwork; text: i18n("Network") }

        Item { Kirigami.FormData.isSection: true }

        QQC2.ComboBox {
            Kirigami.FormData.label: i18n("Disk shows:")
            model: [i18n("Space used"), i18n("Read + write speed")]
            currentIndex: cfg_diskMode === "io" ? 1 : 0
            onActivated: cfg_diskMode = currentIndex === 1 ? "io" : "usage"
        }
        QQC2.TextField {
            id: diskSensor
            Kirigami.FormData.label: i18n("Space sensor:")
            enabled: cfg_diskMode !== "io"
            placeholderText: "disk/all/usedPercent"
            QQC2.ToolTip.text: i18n("A ksystemstats sensor id, e.g. disk/all/usedPercent for all disks, or disk/<id>/usedPercent for one (see 'kstatsviewer --list').")
            QQC2.ToolTip.visible: hovered
        }

        Item { Kirigami.FormData.isSection: true }

        QQC2.CheckBox { id: showGraphs; Kirigami.FormData.label: i18n("Look:"); text: i18n("History graphs") }
        QQC2.CheckBox { id: colorLabels; text: i18n("Coloured labels") }
        QQC2.CheckBox { id: valuesInside; text: i18n("Values inside the graphs (compact)") }
        QQC2.ComboBox {
            Kirigami.FormData.label: i18n("Labels:")
            model: [i18n("One letter (C, M, G…)"), i18n("Full (CPU, MEM…)"), i18n("None (colours only)")]
            currentIndex: cfg_labels === "full" ? 1 : (cfg_labels === "none" ? 2 : 0)
            onActivated: cfg_labels = ["short", "full", "none"][currentIndex]
        }
        RowLayout {
            Kirigami.FormData.label: i18n("Graph width:")
            QQC2.Slider { id: graphWidth; from: 50; to: 250; stepSize: 10 }
            QQC2.Label { text: graphWidth.value + "%" }
        }
        QQC2.SpinBox {
            id: interval
            Kirigami.FormData.label: i18n("Update every (ms):")
            from: 250; to: 10000; stepSize: 250
        }
    }
}
