use serde::{Deserialize, Serialize};
use windows::Win32::Globalization::GetUserDefaultLocaleName;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    PtBr,
    En,
    Es,
    Ru,
}

impl Default for Language {
    fn default() -> Self {
        Self::from_system()
    }
}

impl Language {
    pub const ALL: [Language; 4] = [Language::PtBr, Language::En, Language::Es, Language::Ru];

    pub fn label(self) -> &'static str {
        match self {
            Language::PtBr => "Português",
            Language::En => "English",
            Language::Es => "Español",
            Language::Ru => "Русский",
        }
    }

    pub fn from_system() -> Self {
        let mut buffer = [0u16; 85];
        let length = unsafe { GetUserDefaultLocaleName(&mut buffer) };

        if length <= 0 {
            return Language::En;
        }

        let tag = String::from_utf16_lossy(&buffer[..(length as usize).saturating_sub(1)]);
        match tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase().as_str() {
            "pt" => Language::PtBr,
            "es" => Language::Es,
            "ru" => Language::Ru,
            _ => Language::En,
        }
    }

    fn index(self) -> usize {
        match self {
            Language::PtBr => 0,
            Language::En => 1,
            Language::Es => 2,
            Language::Ru => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    SectionSource,
    SectionFraming,
    SectionPlayback,
    ChooseVideo,
    Preparing,
    NoFile,
    RemoveFromMonitor,
    Converting,
    Zoom,
    Horizontal,
    Vertical,
    Recenter,
    Speed,

    ModeFill,
    ModeFit,
    ModeStretch,
    ModeCenter,
    ModeFree,

    PreviewHint,
    NoVideoChosen,

    Close,
    Apply,
    Saved,
    StartWithWindows,
    CloseToTray,
    PauseFullscreen,
    OptimizeOnImport,
    LanguageLabel,

    PickerTitle,
    SequenceSettings,
    SequenceTitle,
    AddVideo,
    NoThumbnail,
    LibraryEmpty,
    OneImported,
    ManyImported,
    OneInSequence,
    ManyInSequence,
    Save,

    TransitionTitle,
    TransitionNone,
    TransitionFade,
    TransitionLeftRight,
    TransitionRightLeft,
    TransitionTopBottom,
    TransitionBottomTop,
    TransitionRebuild,
    TransitionMorph,

    TimerTitle,
    TimerHint,
    UnitLoops,
    UnitSeconds,
    LoopsOfVideo,
    TimeOnScreen,
    ApplyToAll,
    AllSources,
    NoSources,
    UseSchedule,
    ScheduleHint,
    AddTime,
    Monitor,

    SectionLayers,
    LayerBackground,
    LayersHint,
    PasteIntoComposition,
    SectionResize,
    StretchWidth,
    StretchHeight,
    ResetStretch,
    SectionFilters,
    Brightness,
    Contrast,
    Saturation,
    Temperature,
    ResetFilters,
    FramingHint,
    ResetFraming,

    ConfigSaved,
    ConfigSavedApplied,
    CannotOpenVideo,
    CannotSave,
    CannotChangeStartup,
    PreparingVideo,
    NotOptimized,
    SavedButNoEngine,
    SavedButNoPoster,
    ImportInterrupted,
    TransitionPreviewHint,
    FilterMedia,
    FilterAll,
    PauseBattery,
    PauseEnergySaver,
    Search,
    Favorites,
    Storage,
    Diagnostics,
    RemoveLibrary,
}

impl Text {
    fn row(self) -> [&'static str; 4] {
        use Text::*;
        match self {
            PauseBattery => ["Pausar na bateria", "Pause on battery", "Pausar con batería", "Пауза от батареи"],
            PauseEnergySaver => ["Pausar ao economizar energia", "Pause in energy saver", "Pausar en ahorro de energía", "Пауза при энергосбережении"],
            Search => ["Buscar wallpapers…", "Search wallpapers…", "Buscar fondos…", "Поиск обоев…"],
            Favorites => ["Favoritos", "Favorites", "Favoritos", "Избранное"],
            Storage => ["Espaço e limpeza…", "Storage and cleanup…", "Espacio y limpieza…", "Место и очистка…"],
            Diagnostics => ["Diagnóstico…", "Diagnostics…", "Diagnóstico…", "Диагностика…"],
            RemoveLibrary => ["Remover selecionados da biblioteca", "Remove selected from library", "Quitar seleccionados de la biblioteca", "Удалить выбранные из библиотеки"],
            SectionSource => ["Fonte", "Source", "Fuente", "Источник"],
            SectionFraming => ["Enquadramento", "Framing", "Encuadre", "Кадрирование"],
            SectionPlayback => ["Reprodução", "Playback", "Reproducción", "Воспроизведение"],
            ChooseVideo => ["Escolher vídeo…", "Choose video…", "Elegir vídeo…", "Выбрать видео…"],
            Preparing => ["Preparando o vídeo…", "Preparing video…", "Preparando el vídeo…", "Подготовка видео…"],
            NoFile => ["nenhum arquivo", "no file", "ningún archivo", "нет файла"],
            RemoveFromMonitor => ["Remover deste monitor", "Remove from this monitor", "Quitar de este monitor", "Убрать с этого монитора"],
            Converting => [
                "Convertendo para a resolução do monitor",
                "Converting to the monitor resolution",
                "Convirtiendo a la resolución del monitor",
                "Преобразование под разрешение монитора",
            ],
            Zoom => ["Zoom", "Zoom", "Zoom", "Масштаб"],
            Horizontal => ["Horizontal", "Horizontal", "Horizontal", "По горизонтали"],
            Vertical => ["Vertical", "Vertical", "Vertical", "По вертикали"],
            Recenter => ["Centralizar", "Recenter", "Centrar", "По центру"],
            Speed => ["Velocidade", "Speed", "Velocidad", "Скорость"],

            ModeFill => ["Preencher", "Fill", "Rellenar", "Заполнить"],
            ModeFit => ["Ajustar", "Fit", "Ajustar", "Вписать"],
            ModeStretch => ["Esticar", "Stretch", "Estirar", "Растянуть"],
            ModeCenter => ["Centro", "Center", "Centro", "По центру"],
            ModeFree => ["Livre", "Free", "Libre", "Свободно"],

            PreviewHint => [
                "arraste para mover  ·  roda para ampliar  ·  duplo clique para editar",
                "drag to move  ·  wheel to zoom  ·  double click to edit",
                "arrastra para mover  ·  rueda para ampliar  ·  doble clic para editar",
                "перетащите  ·  колесо для масштаба  ·  двойной клик для правки",
            ],
            NoVideoChosen => [
                "Nenhum vídeo escolhido para este monitor",
                "No video chosen for this monitor",
                "Ningún vídeo elegido para este monitor",
                "Для этого монитора видео не выбрано",
            ],

            Close => ["Fechar", "Close", "Cerrar", "Закрыть"],
            Apply => ["Aplicar", "Apply", "Aplicar", "Применить"],
            Saved => ["Salvo", "Saved", "Guardado", "Сохранено"],
            StartWithWindows => ["Iniciar com o Windows", "Start with Windows", "Iniciar con Windows", "Запускать с Windows"],
            CloseToTray => ["Fechar para a bandeja", "Close to tray", "Cerrar a la bandeja", "Сворачивать в трей"],
            PauseFullscreen => ["Pausar em tela cheia", "Pause on fullscreen", "Pausar en pantalla completa", "Пауза в полноэкранном режиме"],
            OptimizeOnImport => ["Otimizar ao importar", "Optimize on import", "Optimizar al importar", "Оптимизировать при импорте"],
            LanguageLabel => ["Idioma", "Language", "Idioma", "Язык"],

            PickerTitle => ["Escolher vídeo", "Choose video", "Elegir vídeo", "Выбрать видео"],
            SequenceSettings => [
                "Transição e tempo",
                "Transition and timing",
                "Transición y tiempo",
                "Переход и время",
            ],
            SequenceTitle => ["Sequência", "Sequence", "Secuencia", "Последовательность"],
            AddVideo => ["Adicionar vídeo", "Add video", "Añadir vídeo", "Добавить видео"],
            NoThumbnail => ["sem miniatura", "no thumbnail", "sin miniatura", "нет эскиза"],
            LibraryEmpty => [
                "Nenhum vídeo importado ainda. Use o espaço vazio para adicionar o primeiro.",
                "No videos imported yet. Use the empty slot to add the first one.",
                "Aún no hay vídeos importados. Usa el espacio vacío para añadir el primero.",
                "Видео ещё не импортированы. Используйте пустую ячейку, чтобы добавить первое.",
            ],
            OneImported => ["1 vídeo importado", "1 video imported", "1 vídeo importado", "1 видео импортировано"],
            ManyImported => ["{} vídeos importados", "{} videos imported", "{} vídeos importados", "видео импортировано: {}"],
            OneInSequence => ["1 wallpaper na lista", "1 wallpaper in the list", "1 fondo en la lista", "1 обои в списке"],
            ManyInSequence => [
                "{} wallpapers em sequência",
                "{} wallpapers in sequence",
                "{} fondos en secuencia",
                "обоев в последовательности: {}",
            ],
            Save => ["Salvar", "Save", "Guardar", "Сохранить"],

            TransitionTitle => ["Transição", "Transition", "Transición", "Переход"],
            TransitionNone => ["Sem transição", "No transition", "Sin transición", "Без перехода"],
            TransitionFade => ["Fade", "Fade", "Fundido", "Затухание"],
            TransitionLeftRight => ["Esquerda › direita", "Left › right", "Izquierda › derecha", "Слева направо"],
            TransitionRightLeft => ["Direita › esquerda", "Right › left", "Derecha › izquierda", "Справа налево"],
            TransitionTopBottom => ["Cima › baixo", "Top › bottom", "Arriba › abajo", "Сверху вниз"],
            TransitionBottomTop => ["Baixo › cima", "Bottom › top", "Abajo › arriba", "Снизу вверх"],
            TransitionRebuild => ["Reconstrução", "Rebuild", "Reconstrucción", "Сборка"],
            TransitionMorph => ["Transformar", "Morph", "Transformar", "Трансформация"],

            TimerTitle => ["Temporizador", "Timer", "Temporizador", "Таймер"],
            TimerHint => [
                "Quanto tempo cada wallpaper fica na tela antes do próximo.",
                "How long each wallpaper stays on screen before the next one.",
                "Cuánto tiempo permanece cada fondo antes del siguiente.",
                "Сколько времени каждые обои остаются на экране.",
            ],
            UnitLoops => ["Repetições", "Loops", "Repeticiones", "Повторы"],
            UnitSeconds => ["Segundos", "Seconds", "Segundos", "Секунды"],
            LoopsOfVideo => ["Repetições do vídeo", "Video loops", "Repeticiones del vídeo", "Повторов видео"],
            TimeOnScreen => ["Tempo na tela", "Time on screen", "Tiempo en pantalla", "Время на экране"],
            ApplyToAll => ["Definir para todos", "Apply to all", "Definir para todos", "Для всех"],
            AllSources => ["Todas as fontes", "All sources", "Todas las fuentes", "Все источники"],
            NoSources => [
                "Nenhuma fonte na sequência ainda.",
                "No sources in the sequence yet.",
                "Todavía no hay fuentes en la secuencia.",
                "В последовательности пока нет источников.",
            ],
            UseSchedule => ["Usar horários", "Use a schedule", "Usar horarios", "По расписанию"],
            ScheduleHint => [
                "Cada horário vale até o próximo começar, usando o relógio do sistema.",
                "Each time applies until the next one starts, using the system clock.",
                "Cada horario rige hasta que empiece el siguiente, con el reloj del sistema.",
                "Каждое время действует до следующего, по системным часам.",
            ],
            AddTime => ["Adicionar horário", "Add a time", "Añadir horario", "Добавить время"],
            Monitor => ["Monitor", "Monitor", "Monitor", "Монитор"],

            SectionLayers => ["Camadas", "Layers", "Capas", "Слои"],
            LayerBackground => ["Fundo", "Background", "Fondo", "Фон"],
            LayersHint => [
                "Cole itens da biblioteca aqui.",
                "Paste library items here.",
                "Pega elementos de la biblioteca.",
                "Вставьте элементы из библиотеки.",
            ],
            PasteIntoComposition => [
                "Colar na composição",
                "Paste into composition",
                "Pegar en la composición",
                "Вставить в композицию",
            ],
            SectionResize => ["Redimensionar", "Resize", "Redimensionar", "Размер"],
            StretchWidth => ["Largura", "Width", "Ancho", "Ширина"],
            StretchHeight => ["Altura", "Height", "Alto", "Высота"],
            ResetStretch => [
                "Voltar à proporção",
                "Reset proportions",
                "Volver a la proporción",
                "Вернуть пропорции",
            ],
            SectionFilters => ["Filtros", "Filters", "Filtros", "Фильтры"],
            Brightness => ["Brilho", "Brightness", "Brillo", "Яркость"],
            Contrast => ["Contraste", "Contrast", "Contraste", "Контраст"],
            Saturation => ["Saturação", "Saturation", "Saturación", "Насыщенность"],
            Temperature => ["Temperatura", "Temperature", "Temperatura", "Температура"],
            ResetFilters => ["Limpar filtros", "Clear filters", "Limpiar filtros", "Сбросить фильтры"],
            FramingHint => [
                "Botão direito no preview para o enquadramento.",
                "Right-click the preview for framing.",
                "Clic derecho en la vista para el encuadre.",
                "Правый клик по превью — кадрирование.",
            ],
            ResetFraming => ["Redefinir", "Reset", "Restablecer", "Сбросить"],

            ConfigSaved => ["Configuração salva.", "Settings saved.", "Configuración guardada.", "Настройки сохранены."],
            ConfigSavedApplied => [
                "Configuração salva. Wallpaper aplicado.",
                "Settings saved. Wallpaper applied.",
                "Configuración guardada. Fondo aplicado.",
                "Настройки сохранены. Обои применены.",
            ],
            CannotOpenVideo => ["Não consegui abrir o vídeo", "Could not open the video", "No pude abrir el vídeo", "Не удалось открыть видео"],
            CannotSave => ["Não consegui salvar", "Could not save", "No pude guardar", "Не удалось сохранить"],
            CannotChangeStartup => [
                "Não consegui alterar a inicialização",
                "Could not change startup",
                "No pude cambiar el inicio",
                "Не удалось изменить автозапуск",
            ],
            PreparingVideo => ["Preparando o vídeo…", "Preparing the video…", "Preparando el vídeo…", "Подготовка видео…"],
            NotOptimized => ["Sem otimizar", "Not optimized", "Sin optimizar", "Без оптимизации"],
            SavedButNoEngine => [
                "Salvo, mas não consegui iniciar o motor",
                "Saved, but could not start the engine",
                "Guardado, pero no pude iniciar el motor",
                "Сохранено, но не удалось запустить движок",
            ],
            TransitionPreviewHint => [
                "Escolha uma transição para vê-la aqui",
                "Pick a transition to see it here",
                "Elige una transición para verla aquí",
                "Выберите переход, чтобы увидеть его",
            ],
            ImportInterrupted => [
                "A preparação do vídeo foi interrompida.",
                "Video preparation was interrupted.",
                "La preparación del vídeo se interrumpió.",
                "Подготовка видео была прервана.",
            ],
            FilterMedia => ["Vídeos e imagens", "Videos and images", "Vídeos e imágenes", "Видео и изображения"],
            FilterAll => ["Todos os arquivos", "All files", "Todos los archivos", "Все файлы"],
            SavedButNoPoster => [
                "Salvo, mas sem frame estático",
                "Saved, but without a static frame",
                "Guardado, pero sin fotograma estático",
                "Сохранено, но без статичного кадра",
            ],
        }
    }
}

pub fn t(language: Language, key: Text) -> &'static str {
    key.row()[language.index()]
}
