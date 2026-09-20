import React, { createContext, useContext, useState, useEffect } from 'react';

export type Language = 'es' | 'ca' | 'en';

export interface Translations {
  // App Header
  subtitle: string;

  // Step wizard
  stepSource: string;
  stepConfigure: string;
  stepDownload: string;

  // Step 1: Source
  addSourceTitle: string;
  searchPlaceholder: string;
  searchBtn: string;
  analyzingBtn: string;
  directSearchHint: string;
  searchEmptyError: string;
  loadingPhrases: string[];

  // Step 2: SettingsPanel
  exportConfigTitle: string;
  outputFormatTitle: string;
  videoQualityTitle: string;
  audioQualityTitle: string;
  losslessLocked: string;
  losslessLockedDesc: string;
  sampleRateTitle: string;
  cdQuality: string;
  studioQuality: string;

  // Bitrate descriptions
  bitrate128Desc: string;
  bitrate256Desc: string;
  bitrate320Desc: string;
  bitrateLosslessDesc: string;

  // Video resolution descriptions
  videoBestDesc: string;
  video1080pDesc: string;
  video720pDesc: string;
  video480pDesc: string;
  video360pDesc: string;

  // Path & Naming Pattern
  taggingOrgTitle: string;
  pathPatternTitle: string;
  pathPatternDesc: string;
  editCustomPattern: string;
  insertLabel: string;
  livePreviewTitle: string;
  recommendedBadge: string;
  flatBadge: string;

  // Preset Labels
  presetArtistYearAlbumTrack: string;
  presetArtistAlbumTrack: string;
  presetNumberArtistTitle: string;
  presetArtistTitle: string;

  // Tag chip labels
  chipArtist: string;
  chipYear: string;
  chipAlbum: string;
  chipTrackNumber: string;
  chipTitle: string;
  chipTotalTracks: string;
  chipPlaylist: string;
  chipSlash: string;

  // Download Folder
  downloadFolderTitle: string;
  noFolderSet: string;
  loadingFolder: string;
  btnChange: string;
  btnChoose: string;

  // ID3 Tags & Skip
  embedId3Title: string;
  embedId3Desc: string;
  skipMissingTitle: string;
  skipMissingDesc: string;

  // Bot detection bypass
  bypassBotTitle: string;
  bypassBotDesc: string;
  noneBrowserOption: string;
  orPasteCookiesTitle: string;
  orPasteCookiesDesc: string;
  browserExtractMode: string;
  browserExtractDesc: string;
  browserExtractTip: string;

  // Step 3: Track list & Batch actions
  batchOptionsTitle: string;
  batchOptionsDesc: (count: number) => string;
  btnSaveToFolder: string;
  btnSaveAsZip: string;
  saveToFolderTitle: string;
  saveAsZipTitle: string;
  discoveredTracks: string;
  discoveredTrack: string;
  songsParsed: (count: number) => string;
  songParsed: (count: number) => string;

  // Track status
  statusIdle: string;
  statusScraping: string;
  statusDownloading: string;
  statusTranscoding: string;
  statusTagging: string;
  statusCompleted: string;
  statusFailed: string;

  // Progress messages
  downloadedCount: (completed: number, total: number) => string;
  savedToFolderText: (completed: number, total: number) => string;

  // Media Player
  clickToPreview: string;
  closePlayer: string;
  toggleVideo: string;
  youtubeStream: string;
  collapseVideo: string;
  expandVideo: string;

  // Navigation & Actions
  btnBack: string;
  btnContinueToDownload: string;
  btnStartOver: string;
  downloadSong: string;
  clearInput: string;

  // Step 1 & 2
  analyzingCatalogTitle: string;
  foundAndReady: string;
  playlistDefault: string;

  // Step 3 & Progress
  savingDirectlyToFolder: string;
  batchArchivingProgress: string;
  batchArchivingSubtext: string;
  batchFailed: string;

  // Error messages
  errYoutubeBot: string;
  errTrackNotFound: string;
  errForbidden: string;
  errNoDownloadFolder: string;
  errInvalidUrl: string;
  errNetwork: string;
  errFfmpeg: string;
  errYtDlp: string;
  errGeneric: string;
  errFolderSettingsRead: string;
  errFolderStopped: string;

  // Cover Dropzone
  coverDropzoneTitle: string;
  coverDropzonePrompt: string;
  coverDropzoneDefaultHint: string;
  coverDropzoneCustomBadge: string;
  coverDropzoneDefaultBadge: string;
  coverDropzoneResetBtn: string;
  coverDropzoneChangeBtn: string;
  coverDropzoneDownloadBtn: string;
  coverDropzoneApplyToAllLabel: string;
  coverDropzoneApplyToAllHintOn: string;
  coverDropzoneApplyToAllHintOff: string;
  coverDropzoneTrackIndicator: (current: number, total: number) => string;
  coverDropzoneSpectrogramBtn: string;
  coverDropzoneGeneratingSpectrogram: string;
  coverDropzoneSpectrogramTitle: string;
}

const es: Translations = {
  subtitle: 'Extractor de Audio y Video',
  stepSource: 'Añadir Fuente',
  stepConfigure: 'Configurar',
  stepDownload: 'Descargar',

  addSourceTitle: 'Añadir una fuente',
  searchPlaceholder: 'Pega un enlace o busca directamente...',
  searchBtn: 'Buscar',
  analyzingBtn: 'Analizando...',
  directSearchHint:
    'Consejo: Búsqueda directa en YouTube disponible para consultas como artista - canción o álbumes completos.',
  searchEmptyError: 'Por favor, pega un enlace o escribe términos de búsqueda primero.',
  loadingPhrases: [
    'Consultando base de datos oficial...',
    'Sincronizando frecuencias de audio con metadatos...',
    'Descargando carátula en alta fidelidad 600x600px...',
    'Inyectando etiquetas y descriptores de catálogo...',
    'Finalizando buffers de audio...',
    'Ten paciencia, las listas grandes toman unos instantes',
  ],

  exportConfigTitle: '02 / Configuración de Exportación',
  outputFormatTitle: 'Formato de Salida',
  videoQualityTitle: 'Calidad de Video / Resolución',
  audioQualityTitle: 'Calidad de Audio (Bitrate)',
  losslessLocked: 'Sin Pérdida (Lossless)',
  losslessLockedDesc: 'es intrínsecamente sin pérdida e ignora la compresión de bitrate.',
  sampleRateTitle: 'Frecuencia de Muestreo (Sample Rate)',
  cdQuality: 'Calidad CD',
  studioQuality: 'Calidad Estudio',

  bitrate128Desc: 'Calidad estándar, archivo más ligero',
  bitrate256Desc: 'Alta calidad, tamaño equilibrado',
  bitrate320Desc: 'Máxima calidad, óptimo para MP3/M4A',
  bitrateLosslessDesc: 'Calidad master original de estudio',

  videoBestDesc: 'Máxima resolución disponible con audio premium',
  video1080pDesc: 'Transmisión en alta definición Full HD (1920x1080)',
  video720pDesc: 'Transmisión estándar en alta definición HD (1280x720)',
  video480pDesc: 'Calidad estándar (854x480)',
  video360pDesc: 'Calidad compacta optimizada para poco ancho de banda',

  taggingOrgTitle: 'Etiquetado y Organización',
  pathPatternTitle: 'Ruta y Organización de Descargas (Path Pattern)',
  pathPatternDesc:
    'Configura la jerarquía de carpetas y el nombre del archivo. Usa barras (/) para crear subcarpetas automáticamente.',
  editCustomPattern: 'Editar Plantilla al Gusto:',
  insertLabel: 'Insertar:',
  livePreviewTitle: 'Vista Previa de Ruta:',
  recommendedBadge: 'Tu Selección',
  flatBadge: 'Plano',

  presetArtistYearAlbumTrack: 'Artista / Año - Álbum / Pista',
  presetArtistAlbumTrack: 'Artista / Álbum / Pista',
  presetNumberArtistTitle: '01 - Artista - Título',
  presetArtistTitle: 'Artista - Título',

  chipArtist: '+ Artista',
  chipYear: '+ Año',
  chipAlbum: '+ Álbum',
  chipTrackNumber: '+ Nº Pista',
  chipTitle: '+ Título',
  chipTotalTracks: '+ Total Pistas',
  chipPlaylist: '+ Playlist',
  chipSlash: '+ / (Subcarpeta)',

  downloadFolderTitle: 'Carpeta de Descargas',
  noFolderSet: 'Sin carpeta asignada — las descargas estarán bloqueadas hasta que elijas una.',
  loadingFolder: 'Cargando…',
  btnChange: 'Cambiar',
  btnChoose: 'Elegir',

  embedId3Title: 'Incrustar Etiquetas ID3 y Portada',
  embedId3Desc:
    'Incrusta Título, Artista, Álbum, Año, Número de Pista y Portada en alta resolución directamente en los archivos de audio.',
  skipMissingTitle: 'Omitir Pistas no Disponibles',
  skipMissingDesc:
    'Omite automáticamente pistas que fallen o no estén disponibles en lugar de cancelar la descarga en lote.',

  bypassBotTitle: 'Evitar Bloqueo de Bots de YouTube',
  bypassBotDesc: 'Extrae cookies de sesión automáticamente desde tu navegador web activo.',
  noneBrowserOption: '-- Ninguno (Usar pegado manual) --',
  orPasteCookiesTitle: 'O Pega Cookies en Formato Netscape',
  orPasteCookiesDesc:
    'Pega manualmente el texto de tu archivo cookies.txt si la extracción automática de navegador no está disponible.',
  browserExtractMode: 'Modo Extracción Navegador:',
  browserExtractDesc:
    'El motor de descarga extraerá dinámicamente las cookies de tu perfil activo al procesar.',
  browserExtractTip:
    'Consejo: si las descargas fallan al leer cookies, cierra el navegador primero ya que bloquea su base de datos mientras está abierto.',

  batchOptionsTitle: 'Opciones de Descarga en Lote',
  batchOptionsDesc: count =>
    `Exporta las ${count} pistas con etiquetas ID3, portada y tu estructura de carpetas personalizada.`,
  btnSaveToFolder: 'Guardar en Carpeta',
  btnSaveAsZip: 'Guardar como ZIP',
  saveToFolderTitle: 'Guarda todas las pistas organizadas directamente en tu carpeta de descargas',
  saveAsZipTitle: 'Descarga un archivo .ZIP comprimido con todas las pistas organizadas',
  discoveredTracks: 'Pistas Encontradas',
  discoveredTrack: 'Pista Encontrada',
  songsParsed: count => `${count} canciones procesadas con éxito`,
  songParsed: count => `${count} canción procesada con éxito`,

  statusIdle: 'Listo',
  statusScraping: 'Buscando...',
  statusDownloading: 'Descargando...',
  statusTranscoding: 'Convirtiendo...',
  statusTagging: 'Etiquetando...',
  statusCompleted: 'Completado',
  statusFailed: 'Error',

  downloadedCount: (completed, total) => `Descargadas ${completed}/${total} pistas…`,
  savedToFolderText: (completed, total) =>
    `Guardadas ${completed} de ${total} pistas en tu carpeta de descargas configurada.`,

  clickToPreview: 'Clic para previsualizar',
  closePlayer: 'Cerrar Reproductor',
  toggleVideo: 'Mostrar/Ocultar Video',
  youtubeStream: 'Transmisión de YouTube',
  collapseVideo: 'Ocultar video',
  expandVideo: 'Mostrar video',

  btnBack: '← Atrás',
  btnContinueToDownload: 'Continuar a Descargar',
  btnStartOver: 'Empezar de nuevo',
  downloadSong: 'Descargar canción',
  clearInput: 'Limpiar',

  analyzingCatalogTitle: 'Analizando Metadatos del Catálogo',
  foundAndReady: 'Encontrado y listo',
  playlistDefault: 'Lista de reproducción',

  savingDirectlyToFolder: 'Guardando directamente en tu carpeta seleccionada...',
  batchArchivingProgress: 'Convirtiendo, etiquetando y archivando pistas en ZIP...',
  batchArchivingSubtext:
    'Las pistas se etiquetan con metadatos ID3 v2.3, carátula y se organizan en tu archivo.',
  batchFailed: 'La descarga en lote ha fallado',

  errYoutubeBot:
    'YouTube ha bloqueado esta petición por detección de bot. Activa las cookies (perfil del navegador o pegado manual) en Ajustes y reintenta.',
  errTrackNotFound: 'No se ha encontrado ninguna coincidencia en YouTube para esta pista.',
  errForbidden: 'No estás autenticado. Extrae cookies de tu navegador o pégalas en Ajustes.',
  errNoDownloadFolder: 'Elige una carpeta de descargas en Ajustes antes de descargar.',
  errInvalidUrl: 'El enlace no parece válido — por favor, verifica la URL introducida.',
  errNetwork: 'Error de red — comprueba tu conexión a internet e inténtalo de nuevo.',
  errFfmpeg: 'Ha ocurrido un error al procesar, convertir o etiquetar este archivo con FFmpeg.',
  errYtDlp:
    'El descargador ha encontrado un error inesperado al obtener esta pista. Inténtalo más tarde.',
  errGeneric: 'Ha ocurrido un problema inesperado. Por favor, inténtalo de nuevo.',
  errFolderSettingsRead: 'No se han podido leer los ajustes de descarga',
  errFolderStopped: 'Descarga en carpeta detenida',

  coverDropzoneTitle: 'Carátula del Álbum / Descarga',
  coverDropzonePrompt: 'Arrastra una imagen aquí o haz clic para elegir una carátula personalizada',
  coverDropzoneDefaultHint: 'Usando carátula por defecto de Spotify / YouTube',
  coverDropzoneCustomBadge: 'Personalizada',
  coverDropzoneDefaultBadge: 'Original',
  coverDropzoneResetBtn: 'Restablecer original',
  coverDropzoneChangeBtn: 'Cambiar imagen',
  coverDropzoneDownloadBtn: 'Descargar imagen',
  coverDropzoneApplyToAllLabel: 'Incrustar en toda la serie',
  coverDropzoneApplyToAllHintOn: 'Activado: El cambio se aplicará a todas las pistas',
  coverDropzoneApplyToAllHintOff: 'Desactivado: El cambio solo afectará a esta pista',
  coverDropzoneTrackIndicator: (current: number, total: number) => `Pista ${current} de ${total}`,
  coverDropzoneSpectrogramBtn: 'Espectro',
  coverDropzoneGeneratingSpectrogram: 'Generando espectro...',
  coverDropzoneSpectrogramTitle: 'Generar e incrustar espectrograma de audio',
};

const ca: Translations = {
  subtitle: "Extractor d'Àudio i Vídeo",
  stepSource: 'Afegir Font',
  stepConfigure: 'Configurar',
  stepDownload: 'Descarregar',

  addSourceTitle: 'Afegir una font',
  searchPlaceholder: 'Enganxa un enllaç o cerca directament...',
  searchBtn: 'Cercar',
  analyzingBtn: 'Analitzant...',
  directSearchHint:
    'Consell: Cerca directa a YouTube disponible per consultes com artista - cançó o àlbums complets.',
  searchEmptyError: 'Si us plau, enganxa un enllaç o escriu termes de cerca primer.',
  loadingPhrases: [
    'Consultant base de dades oficial...',
    "Sincronitzant freqüències d'àudio amb metadades...",
    'Descarregant portada en alta fidelitat 600x600px...',
    'Injectant etiquetes i descriptors de catàleg...',
    "Finalitzant buffers d'àudio...",
    'Tingues paciència, les llistes grans triguen uns instants',
  ],

  exportConfigTitle: "02 / Configuració d'Exportació",
  outputFormatTitle: 'Format de Sortida',
  videoQualityTitle: 'Qualitat de Vídeo / Resolució',
  audioQualityTitle: "Qualitat d'Àudio (Bitrate)",
  losslessLocked: 'Sense Pèrdua (Lossless)',
  losslessLockedDesc: 'és intrínsecament sense pèrdua i ignora la compressió de bitrate.',
  sampleRateTitle: 'Freqüència de Mostreig (Sample Rate)',
  cdQuality: 'Qualitat CD',
  studioQuality: 'Qualitat Estudi',

  bitrate128Desc: 'Qualitat estàndard, arxiu més lleuger',
  bitrate256Desc: 'Alta qualitat, mida equilibrada',
  bitrate320Desc: 'Màxima qualitat, òptim per a MP3/M4A',
  bitrateLosslessDesc: "Qualitat màster original d'estudi",

  videoBestDesc: 'Màxima resolució disponible amb àudio premium',
  video1080pDesc: 'Transmissió en alta definició Full HD (1920x1080)',
  video720pDesc: 'Transmissió estàndard en alta definició HD (1280x720)',
  video480pDesc: 'Qualitat estàndard (854x480)',
  video360pDesc: 'Qualitat compacta optimitzada per a poca amplada de banda',

  taggingOrgTitle: 'Etiquetatge i Organització',
  pathPatternTitle: 'Ruta i Organització de Descàrregues (Path Pattern)',
  pathPatternDesc:
    "Configura la jerarquia de carpetes i el nom de l'arxiu. Utilitza barres (/) per crear subcarpetes automàticament.",
  editCustomPattern: 'Editar Plantilla al Gust:',
  insertLabel: 'Inserir:',
  livePreviewTitle: 'Vista Prèvia de Ruta:',
  recommendedBadge: 'La Teva Tria',
  flatBadge: 'Pla',

  presetArtistYearAlbumTrack: 'Artista / Any - Àlbum / Pista',
  presetArtistAlbumTrack: 'Artista / Àlbum / Pista',
  presetNumberArtistTitle: '01 - Artista - Títol',
  presetArtistTitle: 'Artista - Títol',

  chipArtist: '+ Artista',
  chipYear: '+ Any',
  chipAlbum: '+ Àlbum',
  chipTrackNumber: '+ Nº Pista',
  chipTitle: '+ Títol',
  chipTotalTracks: '+ Total Pistes',
  chipPlaylist: '+ Playlist',
  chipSlash: '+ / (Subcarpeta)',

  downloadFolderTitle: 'Carpeta de Descàrregues',
  noFolderSet:
    'Sense carpeta assignada — les descàrregues quedaran bloquejades fins que en triïs una.',
  loadingFolder: 'Carregant…',
  btnChange: 'Canviar',
  btnChoose: 'Triar',

  embedId3Title: 'Incrustar Etiquetes ID3 i Portada',
  embedId3Desc:
    "Incrusta Títol, Artista, Àlbum, Any, Número de Pista i Portada d'alta resolució directament als fitxers d'àudio.",
  skipMissingTitle: 'Ometre Pistes no Disponibles',
  skipMissingDesc:
    'Omet automàticament pistes no disponibles o que fallin en lloc de cancel·lar la descàrrega en lot.',

  bypassBotTitle: 'Evitar Bloqueig de Bots de YouTube',
  bypassBotDesc: 'Extreu cookies de sessió automàticament des del teu navegador web actiu.',
  noneBrowserOption: '-- Cap (Utilitzar enganxat manual) --',
  orPasteCookiesTitle: 'O Enganxa Cookies en Format Netscape',
  orPasteCookiesDesc:
    "Enganxa manualment el text del teu fitxer cookies.txt si l'extracció automàtica no està disponible.",
  browserExtractMode: 'Mode Extracció Navegador:',
  browserExtractDesc:
    'El motor de descàrrega extreurà dinàmicament les cookies del teu perfil actiu en processar.',
  browserExtractTip:
    'Consell: si les descàrregues fallen en llegir cookies, tanca el navegador primer ja que bloqueja la seva base de dades mentre està obert.',

  batchOptionsTitle: 'Opcions de Descàrrega en Lot',
  batchOptionsDesc: count =>
    `Exporta les ${count} pistes amb etiquetes ID3, portada i la teva estructura de carpetes personalitzada.`,
  btnSaveToFolder: 'Desar a la Carpeta',
  btnSaveAsZip: 'Desar com a ZIP',
  saveToFolderTitle:
    'Desa totes les pistes organitzades directament a la teva carpeta de descàrregues',
  saveAsZipTitle: 'Descarrega un arxiu .ZIP comprimit amb totes les pistes organitzades',
  discoveredTracks: 'Pistes Trobades',
  discoveredTrack: 'Pista Trobada',
  songsParsed: count => `${count} cançons processades amb èxit`,
  songParsed: count => `${count} cançó processada amb èxit`,

  statusIdle: 'Llest',
  statusScraping: 'Cercant...',
  statusDownloading: 'Descarregant...',
  statusTranscoding: 'Convertint...',
  statusTagging: 'Etiquetant...',
  statusCompleted: 'Completat',
  statusFailed: 'Error',

  downloadedCount: (completed, total) => `Descarregades ${completed}/${total} pistes…`,
  savedToFolderText: (completed, total) =>
    `Desades ${completed} de ${total} pistes a la teva carpeta de descàrregues configurada.`,

  clickToPreview: 'Clic per previsualitzar',
  closePlayer: 'Tancar Reproductor',
  toggleVideo: 'Mostrar/Amagar Vídeo',
  youtubeStream: 'Transmissió de YouTube',
  collapseVideo: 'Amagar vídeo',
  expandVideo: 'Mostrar vídeo',

  btnBack: '← Enrere',
  btnContinueToDownload: 'Continuar a Descarregar',
  btnStartOver: 'Tornar a començar',
  downloadSong: 'Descarregar cançó',
  clearInput: 'Netejar',

  analyzingCatalogTitle: 'Analitzant Metadades del Catàleg',
  foundAndReady: 'Trobat i a punt',
  playlistDefault: 'Llista de reproducció',

  savingDirectlyToFolder: 'Desant directament a la teva carpeta seleccionada...',
  batchArchivingProgress: 'Convertint, etiquetant i arxivant pistes en ZIP...',
  batchArchivingSubtext:
    "Les pistes s'etiqueten amb metadades ID3 v2.3, portada i s'organitzen al teu arxiu.",
  batchFailed: 'La descàrrega en lot ha fallat',

  errYoutubeBot:
    'YouTube ha bloquejat aquesta petició per detecció de bot. Activa les cookies (perfil del navegador o enganxat manual) a Paràmetres i reintenta.',
  errTrackNotFound: "No s'ha trobat cap coincidència a YouTube per a aquesta pista.",
  errForbidden: 'No estàs autenticat. Extreu cookies del teu navegador o enganxa-les a Paràmetres.',
  errNoDownloadFolder: 'Tria una carpeta de descàrregues a Paràmetres abans de descarregar.',
  errInvalidUrl: "L'enllaç no sembla vàlid — si us plau, verifica la URL introduïda.",
  errNetwork: 'Error de xarxa — comprova la teva connexió a internet i torna-ho a intentar.',
  errFfmpeg: 'Hi ha hagut un error en processar, convertir o etiquetar aquest fitxer amb FFmpeg.',
  errYtDlp:
    'El descarregador ha trobat un error inesperat en obtenir aquesta pista. Torna-ho a provar més tard.',
  errGeneric: 'Hi ha hagut un problema inesperat. Si us plau, torna-ho a intentar.',
  errFolderSettingsRead: "No s'han pogut llegir els paràmetres de descàrrega",
  errFolderStopped: 'Descàrrega a la carpeta aturada',

  coverDropzoneTitle: "Portada de l'Àlbum / Descàrrega",
  coverDropzonePrompt: 'Arrossega una imatge aquí o fes clic per triar una portada personalitzada',
  coverDropzoneDefaultHint: 'Utilitzant portada per defecte de Spotify / YouTube',
  coverDropzoneCustomBadge: 'Personalitzada',
  coverDropzoneDefaultBadge: 'Original',
  coverDropzoneResetBtn: 'Restablir original',
  coverDropzoneChangeBtn: 'Canviar imatge',
  coverDropzoneDownloadBtn: 'Descarregar imatge',
  coverDropzoneApplyToAllLabel: 'Incrustar a tota la sèrie',
  coverDropzoneApplyToAllHintOn: "Activat: El canvi s'aplicarà a totes les pistes",
  coverDropzoneApplyToAllHintOff: 'Desactivat: El canvi només afectarà a aquesta pista',
  coverDropzoneTrackIndicator: (current: number, total: number) => `Pista ${current} de ${total}`,
  coverDropzoneSpectrogramBtn: 'Espectre',
  coverDropzoneGeneratingSpectrogram: 'Generant espectre...',
  coverDropzoneSpectrogramTitle: "Generar i incrustar espectrograma d'àudio",
};

const en: Translations = {
  subtitle: 'Audio & Video Extractor',
  stepSource: 'Add Source',
  stepConfigure: 'Configure',
  stepDownload: 'Download',

  addSourceTitle: 'Add a source',
  searchPlaceholder: 'Paste a link or search directly...',
  searchBtn: 'Search',
  analyzingBtn: 'Analyzing...',
  directSearchHint:
    'Tip: Direct YouTube search supported for queries like artist - song title or full albums.',
  searchEmptyError: 'Please paste a link or enter song search terms first.',
  loadingPhrases: [
    'Querying official registry database...',
    'Matching stream frequencies with metadata...',
    'Syncing high-fidelity 600x600px album art...',
    'Injecting catalog tagging descriptors...',
    'Finalizing raw audio buffers...',
    'Hang tight, large playlists take a while',
  ],

  exportConfigTitle: '02 / Export Configuration',
  outputFormatTitle: 'Output Format',
  videoQualityTitle: 'Video Quality / Resolution',
  audioQualityTitle: 'Audio Quality (Bitrate)',
  losslessLocked: 'Lossless Locked',
  losslessLockedDesc: 'is inherently lossless and ignores bitrate compressions.',
  sampleRateTitle: 'Lossless Frequency (Sample Rate)',
  cdQuality: 'CD Quality',
  studioQuality: 'Studio Quality',

  bitrate128Desc: 'Standard quality, smaller file',
  bitrate256Desc: 'High quality, balanced file size',
  bitrate320Desc: 'Extreme quality, best for MP3/M4A',
  bitrateLosslessDesc: 'Original studio master quality',

  videoBestDesc: 'Highest available video feed with premium audio',
  video1080pDesc: '1920x1080 resolution high-definition video stream',
  video720pDesc: '1280x720 standard HD video stream',
  video480pDesc: '854x480 standard definition video stream',
  video360pDesc: '640x360 compact video (optimized for bandwidth)',

  taggingOrgTitle: 'Tagging & Organization',
  pathPatternTitle: 'File & Folder Path Pattern',
  pathPatternDesc:
    'Configure folder hierarchy and filename. Use slashes (/) to create subfolders automatically.',
  editCustomPattern: 'Edit Pattern to Taste:',
  insertLabel: 'Insert:',
  livePreviewTitle: 'Live Path Preview:',
  recommendedBadge: 'Your Choice',
  flatBadge: 'Flat',

  presetArtistYearAlbumTrack: 'Artist / Year - Album / Track',
  presetArtistAlbumTrack: 'Artist / Album / Track',
  presetNumberArtistTitle: '01 - Artist - Title',
  presetArtistTitle: 'Artist - Title',

  chipArtist: '+ Artist',
  chipYear: '+ Year',
  chipAlbum: '+ Album',
  chipTrackNumber: '+ Track #',
  chipTitle: '+ Title',
  chipTotalTracks: '+ Total Tracks',
  chipPlaylist: '+ Playlist',
  chipSlash: '+ / (Subfolder)',

  downloadFolderTitle: 'Download Folder',
  noFolderSet: 'No folder set — downloads will be blocked until you choose one.',
  loadingFolder: 'Loading…',
  btnChange: 'Change',
  btnChoose: 'Choose',

  embedId3Title: 'Embed ID3 Tags & Artwork',
  embedId3Desc:
    'Embeds Title, Artist, Album, Year, Track Number, and High-Res Artwork directly into the audio file headers.',
  skipMissingTitle: 'Skip Missing Tracks',
  skipMissingDesc:
    'Automatically skip tracks that are unavailable or fail to download instead of aborting the batch.',

  bypassBotTitle: 'Bypass YouTube Bot Block',
  bypassBotDesc: 'Extract session cookies automatically from your active browser profile.',
  noneBrowserOption: '-- None (Use Manual Paste) --',
  orPasteCookiesTitle: 'Or Paste Netscape Cookies',
  orPasteCookiesDesc:
    'Manually paste your Netscape cookies file text if browser extraction is not available on your current device setup.',
  browserExtractMode: 'Browser Extract Mode:',
  browserExtractDesc:
    'The download engine will dynamically parse cookies from your active profile on request.',
  browserExtractTip:
    'Tip: if downloads intermittently fail to read cookies, close your browser first — it locks its cookie database while running.',

  batchOptionsTitle: 'Batch Download Options',
  batchOptionsDesc: count =>
    `Export all ${count} tracks with embedded ID3 tags, artwork, and your custom naming pattern.`,
  btnSaveToFolder: 'Save to Folder',
  btnSaveAsZip: 'Save as ZIP',
  saveToFolderTitle: 'Select a directory to save all tagged audio files directly into folders',
  saveAsZipTitle: 'Download a single .ZIP archive containing all tagged tracks and folders',
  discoveredTracks: 'Discovered Tracks',
  discoveredTrack: 'Discovered Track',
  songsParsed: count => `${count} songs parsed successfully`,
  songParsed: count => `${count} song parsed successfully`,

  statusIdle: 'Ready',
  statusScraping: 'Searching...',
  statusDownloading: 'Downloading...',
  statusTranscoding: 'Transcoding...',
  statusTagging: 'Tagging...',
  statusCompleted: 'Completed',
  statusFailed: 'Failed',

  downloadedCount: (completed, total) => `Downloaded ${completed}/${total} tracks…`,
  savedToFolderText: (completed, total) =>
    `Saved ${completed} of ${total} tracks to your configured download folder.`,

  clickToPreview: 'Click to Preview',
  closePlayer: 'Close Player',
  toggleVideo: 'Toggle Video',
  youtubeStream: 'YouTube Stream',
  collapseVideo: 'Collapse video',
  expandVideo: 'Expand video',

  btnBack: '← Back',
  btnContinueToDownload: 'Continue to Download',
  btnStartOver: 'Start over',
  downloadSong: 'Download song',
  clearInput: 'Clear',

  analyzingCatalogTitle: 'Analyzing Catalog Metadata',
  foundAndReady: 'Found and ready',
  playlistDefault: 'Playlist',

  savingDirectlyToFolder: 'Saving directly into your selected folder...',
  batchArchivingProgress: 'Transcoding, tagging, and archiving tracks into ZIP...',
  batchArchivingSubtext:
    'Tracks are tagged with ID3 v2.3 metadata, covers, and organized inside your archive.',
  batchFailed: 'Batch download failed',

  errYoutubeBot:
    'YouTube blocked this as a bot check. Try turning on cookies (browser profile or pasted) in Settings, then retry.',
  errTrackNotFound: "Couldn't find a YouTube match for this track.",
  errForbidden: "You're not authenticated. Extract cookies or paste in settings.",
  errNoDownloadFolder: 'Choose a download folder in Settings before downloading.',
  errInvalidUrl: "That doesn't look like a valid link — double-check the URL.",
  errNetwork: 'Network error — check your connection and try again.',
  errFfmpeg: 'Something went wrong converting or tagging this file.',
  errYtDlp: 'The downloader ran into an unexpected error fetching this track. Try again later.',
  errGeneric: 'Something wrong happened. Try again',
  errFolderSettingsRead: 'Could not read download settings',
  errFolderStopped: 'Folder download stopped',

  coverDropzoneTitle: 'Album Cover / Artwork',
  coverDropzonePrompt: 'Drag an image here or click to select custom artwork',
  coverDropzoneDefaultHint: 'Using default artwork from Spotify / YouTube',
  coverDropzoneCustomBadge: 'Custom',
  coverDropzoneDefaultBadge: 'Original',
  coverDropzoneResetBtn: 'Restore original',
  coverDropzoneChangeBtn: 'Change image',
  coverDropzoneDownloadBtn: 'Download image',
  coverDropzoneApplyToAllLabel: 'Embed in entire series',
  coverDropzoneApplyToAllHintOn: 'Enabled: Changes will apply to all tracks',
  coverDropzoneApplyToAllHintOff: 'Disabled: Changes will only affect this track',
  coverDropzoneTrackIndicator: (current: number, total: number) => `Track ${current} of ${total}`,
  coverDropzoneSpectrogramBtn: 'Spectrum',
  coverDropzoneGeneratingSpectrogram: 'Generating spectrum...',
  coverDropzoneSpectrogramTitle: 'Generate and embed audio spectrogram',
};

const dictionaries: Record<Language, Translations> = { es, ca, en };

export function detectLanguage(): Language {
  if (typeof window === 'undefined') {
    return 'en';
  }
  const saved = localStorage.getItem('sonic_language');
  if (saved === 'es' || saved === 'ca' || saved === 'en') {
    return saved;
  }

  const langs = navigator.languages || [navigator.language];
  for (const l of langs) {
    const lower = l.toLowerCase();
    if (lower.startsWith('ca')) {
      return 'ca';
    }
    if (lower.startsWith('es')) {
      return 'es';
    }
  }
  return 'en';
}

interface I18nContextValue {
  language: Language;
  setLanguage: (lang: Language) => void;
  t: Translations;
}

const I18nContext = createContext<I18nContextValue | null>(null);

export function LanguageProvider({ children }: { children: React.ReactNode }) {
  const [language, setLangState] = useState<Language>(detectLanguage);

  const setLanguage = (lang: Language) => {
    setLangState(lang);
    try {
      localStorage.setItem('sonic_language', lang);
    } catch {
      // ignore
    }
  };

  useEffect(() => {
    try {
      localStorage.setItem('sonic_language', language);
    } catch {
      // ignore
    }
  }, [language]);

  const t = dictionaries[language];

  return React.createElement(
    I18nContext.Provider,
    { value: { language, setLanguage, t } },
    children
  );
}

export function useI18n(): I18nContextValue {
  const ctx = useContext(I18nContext);
  if (!ctx) {
    // Fallback if rendered outside provider
    const lang = detectLanguage();
    return {
      language: lang,
      setLanguage: () => {},
      t: dictionaries[lang],
    };
  }
  return ctx;
}
