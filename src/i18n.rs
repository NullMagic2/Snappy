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
    translated(key, language())
}

fn translated(key: &str, language: Language) -> &str {
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
        "Mouse pointer" => [
            "Mouse pointer",
            "Puntero del ratón",
            "Δείκτης ποντικιού",
            "Ponteiro do mouse",
            "Ponteiro do rato",
        ],
        "Include pointer in captures: On" => [
            "Include pointer in captures: On",
            "Incluir puntero en las capturas: activado",
            "Συμπερίληψη δείκτη στις λήψεις: ενεργή",
            "Incluir ponteiro nas capturas: ligado",
            "Incluir ponteiro nas capturas: ligado",
        ],
        "Include pointer in captures: Off" => [
            "Include pointer in captures: Off",
            "Incluir puntero en las capturas: desactivado",
            "Συμπερίληψη δείκτη στις λήψεις: ανενεργή",
            "Incluir ponteiro nas capturas: desligado",
            "Incluir ponteiro nas capturas: desligado",
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
        "Dropped frames" => [
            "Dropped frames",
            "Fotogramas perdidos",
            "Χαμένα καρέ",
            "Quadros perdidos",
            "Fotogramas perdidos",
        ],
        "of" => ["of", "de", "από", "de", "de"],
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
        "Drag to move" => [
            "Drag to move",
            "Arrastra para mover",
            "Σύρετε για μετακίνηση",
            "Arraste para mover",
            "Arraste para mover",
        ],
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
        "Native resolution · source frame rate · lossless" => [
            "Native resolution · source frame rate · lossless",
            "Resolución original · frecuencia de la fuente · sin pérdida",
            "Αρχική ανάλυση · ρυθμός καρέ πηγής · χωρίς απώλειες",
            "Resolução nativa · taxa de quadros original · sem perdas",
            "Resolução nativa · taxa de fotogramas original · sem perdas",
        ],
        "source frame rate" => [
            "source frame rate",
            "frecuencia de la fuente",
            "ρυθμός καρέ πηγής",
            "taxa de quadros original",
            "taxa de fotogramas original",
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
        "Use" => ["Use", "Usar", "Χρήση", "Usar", "Usar"],
        "Default folder" => [
            "Default folder",
            "Carpeta predeterminada",
            "Προεπιλεγμένος φάκελος",
            "Pasta padrão",
            "Pasta predefinida",
        ],
        "Close" => ["Close", "Cerrar", "Κλείσιμο", "Fechar", "Fechar"],
        "Could not complete capture" => [
            "Could not complete capture",
            "No se pudo completar la captura",
            "Δεν ήταν δυνατή η ολοκλήρωση της λήψης",
            "Não foi possível concluir a captura",
            "Não foi possível concluir a captura",
        ],
        "Could not calculate size" => [
            "Could not calculate size",
            "No se pudo calcular el tamaño",
            "Δεν ήταν δυνατός ο υπολογισμός μεγέθους",
            "Não foi possível calcular o tamanho",
            "Não foi possível calcular o tamanho",
        ],
        "Approximate size per minute. Recording continues until Stop is pressed." => [
            "Approximate size per minute. Recording continues until Stop is pressed.",
            "Tamaño aproximado por minuto. La grabación continúa hasta pulsar Detener.",
            "Μέγεθος κατά προσέγγιση ανά λεπτό. Η εγγραφή συνεχίζεται μέχρι να πατήσετε Διακοπή.",
            "Tamanho aproximado por minuto. A gravação continua até você pressionar Parar.",
            "Tamanho aproximado por minuto. A gravação continua até premir Parar.",
        ],
        "Registering Print Screen…" => [
            "Registering Print Screen…",
            "Registrando Impr Pant…",
            "Καταχώριση Print Screen…",
            "Registrando Print Screen…",
            "A registar Print Screen…",
        ],
        "Print Screen: managed by the desktop" => [
            "Print Screen: managed by the desktop",
            "Impr Pant: gestionado por el escritorio",
            "Print Screen: διαχείριση από την επιφάνεια εργασίας",
            "Print Screen: gerenciado pelo ambiente de trabalho",
            "Print Screen: gerido pelo ambiente de trabalho",
        ],
        "Print Screen: ready" => [
            "Print Screen: ready",
            "Impr Pant: listo",
            "Print Screen: έτοιμο",
            "Print Screen: pronto",
            "Print Screen: pronto",
        ],
        "Capture shortcut" => [
            "Capture shortcut",
            "Atajo de captura",
            "Συντόμευση λήψης",
            "Atalho de captura",
            "Atalho de captura",
        ],
        "Bind Print Screen to snappy --capture in system Keyboard Shortcuts." => [
            "Bind Print Screen to snappy --capture in system Keyboard Shortcuts.",
            "Asigna Impr Pant a snappy --capture en los atajos de teclado del sistema.",
            "Αντιστοιχίστε το Print Screen στο snappy --capture στις συντομεύσεις πληκτρολογίου του συστήματος.",
            "Associe Print Screen a snappy --capture nos atalhos de teclado do sistema.",
            "Associe Print Screen a snappy --capture nos atalhos de teclado do sistema.",
        ],
        "Open Snappy" => [
            "Open Snappy",
            "Abrir Snappy",
            "Άνοιγμα Snappy",
            "Abrir Snappy",
            "Abrir Snappy",
        ],
        "System shortcut" => [
            "System shortcut",
            "Atajo del sistema",
            "Συντόμευση συστήματος",
            "Atalho do sistema",
            "Atalho do sistema",
        ],
        "Global shortcut portal unavailable" => [
            "Global shortcut portal unavailable",
            "Portal de atajos globales no disponible",
            "Η πύλη καθολικών συντομεύσεων δεν είναι διαθέσιμη",
            "Portal de atalhos globais indisponível",
            "Portal de atalhos globais indisponível",
        ],
        "Print Screen is already assigned to another application. Change its system shortcut to use Snappy." => {
            [
                "Print Screen is already assigned to another application. Change its system shortcut to use Snappy.",
                "Impr Pant ya está asignado a otra aplicación. Cambia el atajo del sistema para usar Snappy.",
                "Το Print Screen έχει ήδη αντιστοιχιστεί σε άλλη εφαρμογή. Αλλάξτε τη συντόμευση συστήματος για να χρησιμοποιήσετε το Snappy.",
                "Print Screen já está atribuído a outro aplicativo. Altere o atalho do sistema para usar o Snappy.",
                "Print Screen já está atribuído a outra aplicação. Altere o atalho do sistema para usar o Snappy.",
            ]
        }
        "Cannot register Print Screen without an X11 display" => [
            "Cannot register Print Screen without an X11 display",
            "No se puede registrar Impr Pant sin una pantalla X11",
            "Δεν είναι δυνατή η καταχώριση του Print Screen χωρίς οθόνη X11",
            "Não é possível registrar Print Screen sem uma tela X11",
            "Não é possível registar Print Screen sem um ecrã X11",
        ],
        "The screen layout changed. Please capture again." => [
            "The screen layout changed. Please capture again.",
            "La disposición de las pantallas cambió. Vuelve a capturar.",
            "Η διάταξη των οθονών άλλαξε. Κάντε ξανά λήψη.",
            "A disposição das telas mudou. Capture novamente.",
            "A disposição dos ecrãs mudou. Capture novamente.",
        ],
        "GNOME is still running an older Snappy integration. Install the latest Snappy package, then sign out and back in to load it." => {
            [
                "GNOME is still running an older Snappy integration. Install the latest Snappy package, then sign out and back in to load it.",
                "GNOME sigue usando una integración antigua de Snappy. Instala el paquete más reciente y cierra e inicia sesión para cargarlo.",
                "Το GNOME εκτελεί ακόμη παλαιότερη ενσωμάτωση του Snappy. Εγκαταστήστε το πιο πρόσφατο πακέτο και αποσυνδεθείτε και συνδεθείτε ξανά για να φορτωθεί.",
                "O GNOME ainda está usando uma integração antiga do Snappy. Instale o pacote mais recente e saia e entre na sessão para carregá-lo.",
                "O GNOME ainda está a usar uma integração antiga do Snappy. Instale o pacote mais recente e termine e volte a iniciar a sessão para o carregar.",
            ]
        }
        "Unsupported Snappy GNOME Shell integration version" => [
            "Unsupported Snappy GNOME Shell integration version",
            "Versión no compatible de la integración de Snappy con GNOME Shell",
            "Μη υποστηριζόμενη έκδοση ενσωμάτωσης Snappy στο GNOME Shell",
            "Versão não suportada da integração do Snappy com GNOME Shell",
            "Versão não suportada da integração do Snappy com GNOME Shell",
        ],
        "Shell returned an invalid screenshot" => [
            "Shell returned an invalid screenshot",
            "El entorno devolvió una captura no válida",
            "Το περιβάλλον επέστρεψε μη έγκυρο στιγμιότυπο",
            "O ambiente retornou uma captura inválida",
            "O ambiente devolveu uma captura inválida",
        ],
        "The desktop did not provide a screen stream" => [
            "The desktop did not provide a screen stream",
            "El escritorio no proporcionó un flujo de pantalla",
            "Η επιφάνεια εργασίας δεν παρείχε ροή οθόνης",
            "O ambiente não forneceu um fluxo de tela",
            "O ambiente não forneceu um fluxo de ecrã",
        ],
        "Portal returned a non-local screenshot" => [
            "Portal returned a non-local screenshot",
            "El portal devolvió una captura no local",
            "Η πύλη επέστρεψε μη τοπικό στιγμιότυπο",
            "O portal retornou uma captura não local",
            "O portal devolveu uma captura não local",
        ],
        "Unknown screenshot format" => [
            "Unknown screenshot format",
            "Formato de captura desconocido",
            "Άγνωστη μορφή στιγμιότυπου",
            "Formato de captura desconhecido",
            "Formato de captura desconhecido",
        ],
        "Capture backend did not finish in time" => [
            "Capture backend did not finish in time",
            "El sistema de captura no terminó a tiempo",
            "Το σύστημα λήψης δεν ολοκληρώθηκε εγκαίρως",
            "O sistema de captura não terminou a tempo",
            "O sistema de captura não terminou a tempo",
        ],
        "No X11 display. Start in a graphical session." => [
            "No X11 display. Start in a graphical session.",
            "No hay pantalla X11. Inicia una sesión gráfica.",
            "Δεν υπάρχει οθόνη X11. Εκκινήστε σε γραφική συνεδρία.",
            "Nenhuma tela X11. Inicie em uma sessão gráfica.",
            "Nenhum ecrã X11. Inicie numa sessão gráfica.",
        ],
        "Cannot open the X11 display" => [
            "Cannot open the X11 display",
            "No se puede abrir la pantalla X11",
            "Δεν είναι δυνατό το άνοιγμα της οθόνης X11",
            "Não é possível abrir a tela X11",
            "Não é possível abrir o ecrã X11",
        ],
        "Capture folder must be an absolute path" => [
            "Capture folder must be an absolute path",
            "La carpeta de captura debe ser una ruta absoluta",
            "Ο φάκελος λήψης πρέπει να είναι απόλυτη διαδρομή",
            "A pasta de captura deve ter um caminho absoluto",
            "A pasta de captura deve ter um caminho absoluto",
        ],
        "Capture folder must be UTF-8" => [
            "Capture folder must be UTF-8",
            "La carpeta de captura debe usar UTF-8",
            "Ο φάκελος λήψης πρέπει να χρησιμοποιεί UTF-8",
            "A pasta de captura deve usar UTF-8",
            "A pasta de captura deve usar UTF-8",
        ],
        "Capture has no parent folder" => [
            "Capture has no parent folder",
            "La captura no tiene carpeta contenedora",
            "Η λήψη δεν έχει γονικό φάκελο",
            "A captura não tem pasta de destino",
            "A captura não tem pasta de destino",
        ],
        "Too many captures with the same timestamp" => [
            "Too many captures with the same timestamp",
            "Demasiadas capturas con la misma fecha y hora",
            "Πάρα πολλές λήψεις με την ίδια χρονική σήμανση",
            "Muitas capturas com a mesma data e hora",
            "Demasiadas capturas com a mesma data e hora",
        ],
        "Cannot save screenshot" => [
            "Cannot save screenshot",
            "No se puede guardar la captura",
            "Δεν είναι δυνατή η αποθήκευση του στιγμιότυπου",
            "Não é possível salvar a captura",
            "Não é possível guardar a captura",
        ],
        "Cannot create output folder" => [
            "Cannot create output folder",
            "No se puede crear la carpeta de destino",
            "Δεν είναι δυνατή η δημιουργία φακέλου προορισμού",
            "Não é possível criar a pasta de destino",
            "Não é possível criar a pasta de destino",
        ],
        "Cannot save capture" => [
            "Cannot save capture",
            "No se puede guardar la captura",
            "Δεν είναι δυνατή η αποθήκευση της λήψης",
            "Não é possível salvar a captura",
            "Não é possível guardar a captura",
        ],
        "Cannot read portal screenshot" => [
            "Cannot read portal screenshot",
            "No se puede leer la captura del portal",
            "Δεν είναι δυνατή η ανάγνωση του στιγμιότυπου της πύλης",
            "Não é possível ler a captura do portal",
            "Não é possível ler a captura do portal",
        ],
        "Cannot start capture backend" => [
            "Cannot start capture backend",
            "No se puede iniciar el sistema de captura",
            "Δεν είναι δυνατή η εκκίνηση του συστήματος λήψης",
            "Não é possível iniciar o sistema de captura",
            "Não é possível iniciar o sistema de captura",
        ],
        "Capture failed" => [
            "Capture failed",
            "La captura falló",
            "Η λήψη απέτυχε",
            "A captura falhou",
            "A captura falhou",
        ],
        "Cannot start recorder" => [
            "Cannot start recorder",
            "No se puede iniciar la grabación",
            "Δεν είναι δυνατή η έναρξη εγγραφής",
            "Não é possível iniciar a gravação",
            "Não é possível iniciar a gravação",
        ],
        "Recording failed" => [
            "Recording failed",
            "La grabación falló",
            "Η εγγραφή απέτυχε",
            "A gravação falhou",
            "A gravação falhou",
        ],
        "Cannot save recording" => [
            "Cannot save recording",
            "No se puede guardar la grabación",
            "Δεν είναι δυνατή η αποθήκευση της εγγραφής",
            "Não é possível salvar a gravação",
            "Não é possível guardar a gravação",
        ],
        "Cannot export" => [
            "Cannot export",
            "No se puede exportar",
            "Δεν είναι δυνατή η εξαγωγή",
            "Não é possível exportar",
            "Não é possível exportar",
        ],
        "Export failed" => [
            "Export failed",
            "La exportación falló",
            "Η εξαγωγή απέτυχε",
            "A exportação falhou",
            "A exportação falhou",
        ],
        "Cannot register Snappy with the desktop portal" => [
            "Cannot register Snappy with the desktop portal",
            "No se puede registrar Snappy en el portal del escritorio",
            "Δεν είναι δυνατή η εγγραφή του Snappy στην πύλη επιφάνειας εργασίας",
            "Não é possível registrar o Snappy no portal do ambiente",
            "Não é possível registar o Snappy no portal do ambiente",
        ],
        "Snappy GNOME Shell integration is not ready. Sign out and back in once after installation, then enable Snappy Capture Integration in Extensions." => {
            [
                "Snappy GNOME Shell integration is not ready. Sign out and back in once after installation, then enable Snappy Capture Integration in Extensions.",
                "La integración de Snappy con GNOME Shell no está lista. Cierra e inicia sesión tras instalar y activa Snappy Capture Integration en Extensiones.",
                "Η ενσωμάτωση Snappy στο GNOME Shell δεν είναι έτοιμη. Αποσυνδεθείτε και συνδεθείτε ξανά μετά την εγκατάσταση και ενεργοποιήστε το Snappy Capture Integration στις Επεκτάσεις.",
                "A integração do Snappy com GNOME Shell não está pronta. Saia e entre na sessão após a instalação e ative Snappy Capture Integration em Extensões.",
                "A integração do Snappy com GNOME Shell não está pronta. Termine e volte a iniciar a sessão após a instalação e ative Snappy Capture Integration nas Extensões.",
            ]
        }
        "The desktop portal could not complete capture. Check that its backend is installed, then try again." => {
            [
                "The desktop portal could not complete capture. Check that its backend is installed, then try again.",
                "El portal del escritorio no pudo completar la captura. Comprueba que su componente esté instalado e inténtalo de nuevo.",
                "Η πύλη επιφάνειας εργασίας δεν μπόρεσε να ολοκληρώσει τη λήψη. Ελέγξτε ότι έχει εγκατασταθεί το σύστημα υποστήριξής της και δοκιμάστε ξανά.",
                "O portal do ambiente não conseguiu concluir a captura. Verifique se o componente necessário está instalado e tente novamente.",
                "O portal do ambiente não conseguiu concluir a captura. Verifique se o componente necessário está instalado e tente novamente.",
            ]
        }
        "Original recording kept in" => [
            "Original recording kept in",
            "Grabación original conservada en",
            "Η αρχική εγγραφή διατηρήθηκε στο",
            "Gravação original mantida em",
            "Gravação original guardada em",
        ],
        "Capture source is not a native stream" => [
            "Capture source is not a native stream",
            "La fuente de captura no es un flujo nativo",
            "Η πηγή λήψης δεν είναι εγγενής ροή",
            "A fonte de captura não é um fluxo nativo",
            "A fonte de captura não é um fluxo nativo",
        ],
        "A screenshot is not a recording source" => [
            "A screenshot is not a recording source",
            "Una captura de imagen no es una fuente de grabación",
            "Ένα στιγμιότυπο δεν είναι πηγή εγγραφής",
            "Uma captura de imagem não é uma fonte de gravação",
            "Uma captura de imagem não é uma fonte de gravação",
        ],
        _ => return key,
    };
    catalog[language as usize]
}

/// Translate application messages while keeping filenames and backend diagnostics intact.
pub fn error(message: &str) -> String {
    let translated = tr(message);
    if translated != message {
        return translated.into();
    }
    if let Some((heading, detail)) = message.split_once(": ") {
        return format!("{}: {detail}", tr(heading));
    }
    message.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_strings_have_all_five_translations() {
        // Check the actual static UI lookups, including dynamic pointer tooltips.
        for source in [
            include_str!("ui.rs"),
            include_str!("selection.rs"),
            include_str!("shortcut.rs"),
            include_str!("preset.rs"),
        ] {
            for tail in source.split("tr(\"").skip(1) {
                let key = tail.split('"').next().unwrap();
                for language in Language::ALL.into_iter().skip(1) {
                    let value = translated(key, language);
                    assert!(!value.is_empty(), "Missing {key}");
                    assert!(
                        value != key || ["Video", "Format", "OK", "On", "Off"].contains(&key),
                        "Untranslated {key} in {language:?}"
                    );
                }
            }
        }
    }
}
