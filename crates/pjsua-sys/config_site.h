/* Reference copy of pjproject's pjlib/include/pj/config_site.h, which
 * the fork's .gitignore keeps out of git. Copy it there and rebuild
 * pjproject (see CLAUDE.md) when setting up a machine.
 * PJMEDIA_HAS_LIBYUV must be 1: without a format converter no camera's
 * frames reach the H.264 encoder and video calls send nothing. */
#define PJMEDIA_HAS_VIDEO             1
#define PJMEDIA_HAS_OPENH264_CODEC    1
#define PJMEDIA_HAS_LIBYUV            1    // format conversion for video capture (YUY2 -> I420, scaling)
#define PJMEDIA_VIDEO_DEV_HAS_SDL     1
#define PJMEDIA_VIDEO_DEV_HAS_DSHOW   1
#define PJMEDIA_HAS_OPENCORE_AMRNB_CODEC 1
#define PJMEDIA_HAS_OPENCORE_AMRWB_CODEC 1
#define PJMEDIA_HAS_FFMPEG            0
#define PJMEDIA_HAS_VPX_CODEC         0    //by default VP8 codec is enabled
#define PJMEDIA_HAS_VPX_CODEC_VP9     0    //enable VP9 codec
#define PJ_HAS_SSL_SOCK 1
#define PJMEDIA_TXT_STREAM_MAX_RED_LEVELS 0