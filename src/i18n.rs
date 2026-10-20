//! Module: i18n. Purpose: shared interface translations and language selection.
//! Created: 2026-10-19. Architecture: one catalog serves the toolbar, dialogs and presets.
use std::sync::atomic::{AtomicU8, Ordering};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    English,
    Spanish,
    Greek,
    PortugueseBrazil,
    PortuguesePortugal,
}
impl Language {
    pub const ALL: [Self; 5] = [
        Self::English,
        Self::Spanish,
        Self::Greek,
        Self::PortugueseBrazil,
        Self::PortuguesePortugal,
    ];
    pub fn code(self) -> &'static str {
        ["en", "es", "el", "pt-BR", "pt-PT"][self as usize]
    }
    pub fn name(self) -> &'static str {
        [
            "English",
            "Español",
            "Ελληνικά",
            "Português (Brasil)",
            "Português (Portugal)",
        ][self as usize]
    }
    pub fn from_code(code: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|lang| lang.code() == code)
            .unwrap_or(Self::English)
    }
}
static CURRENT: AtomicU8 = AtomicU8::new(0);
pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}
pub fn language() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed) as usize]
}
pub fn tr(key: &str) -> &str {
    let catalog = match key {
        "Screenshot" => [
            "Screenshot",
            "Captura de pantalla",
            "Στιγμιότυπο",
            "Captura de tela",
            "Captura de ecrã",
        ],
        "Video" => ["Video", "Vídeo", "Βίντεο", "Vídeo", "Vídeo"],
        "Selection" => ["Selection", "Selección", "Επιλογή", "Seleção", "Seleção"],
        "Screen" => ["Screen", "Pantalla", "Οθόνη", "Tela", "Ecrã"],
        "Capture" => ["Capture", "Capturar", "Λήψη", "Capturar", "Capturar"],
        "Record" => ["Record", "Grabar", "Εγγραφή", "Gravar", "Gravar"],
        "Stop" => ["Stop", "Detener", "Διακοπή", "Parar", "Parar"],
        "Settings" => [
            "Settings",
            "Ajustes",
            "Ρυθμίσεις",
            "Configurações",
            "Definições",
        ],
        "Snappy settings" => [
            "Snappy settings",
            "Ajustes de Snappy",
            "Ρυθμίσεις Snappy",
            "Configurações do Snappy",
            "Definições do Snappy",
        ],
        "Cancel" => ["Cancel", "Cancelar", "Ακύρωση", "Cancelar", "Cancelar"],
        "Save" => ["Save", "Guardar", "Αποθήκευση", "Salvar", "Guardar"],
        "Copy" => ["Copy", "Copiar", "Αντιγραφή", "Copiar", "Copiar"],
        "Copied" => ["Copied", "Copiado", "Αντιγράφηκε", "Copiado", "Copiado"],
        "OK" => ["OK", "Aceptar", "Εντάξει", "OK", "OK"],
        "Done" => ["Done", "Hecho", "Τέλος", "Concluído", "Concluído"],
        "Quit Snappy" => [
            "Quit Snappy",
            "Salir de Snappy",
            "Έξοδος από το Snappy",
            "Sair do Snappy",
            "Sair do Snappy",
        ],
        "Desktop sound" => [
            "Desktop sound",
            "Sonido del escritorio",
            "Ήχος επιφάνειας εργασίας",
            "Áudio do sistema",
            "Áudio do sistema",
        ],
        "Desktop sound: On (video)" => [
            "Desktop sound: On (video)",
            "Sonido del escritorio: activado (vídeo)",
            "Ήχος επιφάνειας εργασίας: ενεργός (βίντεο)",
            "Áudio do sistema: ligado (vídeo)",
            "Áudio do sistema: ligado (vídeo)",
        ],
        "Desktop sound: Off (video)" => [
            "Desktop sound: Off (video)",
            "Sonido del escritorio: desactivado (vídeo)",
            "Ήχος επιφάνειας εργασίας: ανενεργός (βίντεο)",
            "Áudio do sistema: desligado (vídeo)",
            "Áudio do sistema: desligado (vídeo)",
        ],
        "On" => ["On", "Activado", "Ενεργό", "Ligado", "Ligado"],
        "Off" => ["Off", "Desactivado", "Ανενεργό", "Desligado", "Desligado"],
        "Images" => ["Images", "Imágenes", "Εικόνες", "Imagens", "Imagens"],
        "Videos" => ["Videos", "Vídeos", "Βίντεο", "Vídeos", "Vídeos"],
        "Default" => [
            "Default",
            "Predeterminada",
            "Προεπιλογή",
            "Padrão",
            "Predefinida",
        ],
        "Folder" => ["Folder", "Carpeta", "Φάκελος", "Pasta", "Pasta"],
        "Select folder" => [
            "Select folder",
            "Seleccionar carpeta",
            "Επιλογή φακέλου",
            "Selecionar pasta",
            "Selecionar pasta",
        ],
        "Video quality" => [
            "Video quality",
            "Calidad de vídeo",
            "Ποιότητα βίντεο",
            "Qualidade do vídeo",
            "Qualidade do vídeo",
        ],
        "Capture saved" => [
            "Capture saved",
            "Captura guardada",
            "Η λήψη αποθηκεύτηκε",
            "Captura salva",
            "Captura guardada",
        ],
        "Save screenshot" => [
            "Save screenshot",
            "Guardar captura",
            "Αποθήκευση στιγμιότυπου",
            "Salvar captura",
            "Guardar captura",
        ],
        "Saving…" => [
            "Saving…",
            "Guardando…",
            "Αποθήκευση…",
            "Salvando…",
            "A guardar…",
        ],
        "Language" => ["Language", "Idioma", "Γλώσσα", "Idioma", "Idioma"],
        "Shortcut" => ["Shortcut", "Atajo", "Συντόμευση", "Atalho", "Atalho"],
        "Size" => ["Size", "Tamaño", "Μέγεθος", "Tamanho", "Tamanho"],
        "Estimated size" => [
            "Estimated size",
            "Tamaño estimado",
            "Εκτιμώμενο μέγεθος",
            "Tamanho estimado",
            "Tamanho estimado",
        ],
        "Format" => ["Format", "Formato", "Μορφή", "Formato", "Formato"],
        "File name" => [
            "File name",
            "Nombre de archivo",
            "Όνομα αρχείου",
            "Nome do arquivo",
            "Nome do ficheiro",
        ],
        "Calculating size preview…" => [
            "Calculating size preview…",
            "Calculando tamaño…",
            "Υπολογισμός μεγέθους…",
            "Calculando tamanho…",
            "A calcular tamanho…",
        ],
        "per minute" => [
            "per minute",
            "por minuto",
            "ανά λεπτό",
            "por minuto",
            "por minuto",
        ],
        "for" => ["for", "para", "για", "para", "para"],
        "Up to" => ["Up to", "Hasta", "Έως", "Até", "Até"],
        "Native resolution · 30 fps · lossless" => [
            "Native resolution · 30 fps · lossless",
            "Resolución original · 30 fps · sin pérdida",
            "Αρχική ανάλυση · 30 fps · χωρίς απώλειες",
            "Resolução nativa · 30 fps · sem perdas",
            "Resolução nativa · 30 fps · sem perdas",
        ],
        "Discord-friendly · MP4 · balanced" => [
            "Discord-friendly · MP4 · balanced",
            "Para Discord · MP4 · equilibrado",
            "Για Discord · MP4 · ισορροπημένο",
            "Para Discord · MP4 · equilibrado",
            "Para Discord · MP4 · equilibrado",
        ],
        "Small file · MP4 · compact" => [
            "Small file · MP4 · compact",
            "Archivo pequeño · MP4 · compacto",
            "Μικρό αρχείο · MP4 · συμπαγές",
            "Arquivo pequeno · MP4 · compacto",
            "Ficheiro pequeno · MP4 · compacto",
        ],
        "High quality · MP4 · sharp" => [
            "High quality · MP4 · sharp",
            "Alta calidad · MP4 · nítido",
            "Υψηλή ποιότητα · MP4 · ευκρινές",
            "Alta qualidade · MP4 · nítido",
            "Alta qualidade · MP4 · nítido",
        ],
        "Full quality · MKV · lossless" => [
            "Full quality · MKV · lossless",
            "Calidad completa · MKV · sin pérdida",
            "Πλήρης ποιότητα · MKV · χωρίς απώλειες",
            "Qualidade total · MKV · sem perdas",
            "Qualidade total · MKV · sem perdas",
        ],
        "Full quality · PNG · lossless" => [
            "Full quality · PNG · lossless",
            "Calidad completa · PNG · sin pérdida",
            "Πλήρης ποιότητα · PNG · χωρίς απώλειες",
            "Qualidade total · PNG · sem perdas",
            "Qualidade total · PNG · sem perdas",
        ],
        "Balanced · JPEG · quality 90" => [
            "Balanced · JPEG · quality 90",
            "Equilibrado · JPEG · calidad 90",
            "Ισορροπημένο · JPEG · ποιότητα 90",
            "Equilibrado · JPEG · qualidade 90",
            "Equilibrado · JPEG · qualidade 90",
        ],
        "Small file · JPEG · quality 70" => [
            "Small file · JPEG · quality 70",
            "Archivo pequeño · JPEG · calidad 70",
            "Μικρό αρχείο · JPEG · ποιότητα 70",
            "Arquivo pequeno · JPEG · qualidade 70",
            "Ficheiro pequeno · JPEG · qualidade 70",
        ],
        "Bitmap · BMP · uncompressed" => [
            "Bitmap · BMP · uncompressed",
            "Mapa de bits · BMP · sin comprimir",
            "Bitmap · BMP · ασυμπίεστο",
            "Bitmap · BMP · sem compressão",
            "Bitmap · BMP · sem compressão",
        ],
        "Still image · GIF · up to 256 colors" => [
            "Still image · GIF · up to 256 colors",
            "Imagen fija · GIF · hasta 256 colores",
            "Στατική εικόνα · GIF · έως 256 χρώματα",
            "Imagem estática · GIF · até 256 cores",
            "Imagem estática · GIF · até 256 cores",
        ],
        _ => return key,
    };
    catalog[language() as usize]
}
