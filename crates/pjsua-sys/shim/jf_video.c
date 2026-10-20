/*
 * Video taps for drawing video inside RusTone's window: a sink port in
 * pjsua's video conference bridge that asks for BGRA frames of a fixed
 * size (the bridge converts and scales into it, keeping the aspect ratio)
 * and hands each frame to a callback.
 */

#include <pjsua-lib/pjsua.h>

typedef void (*jf_video_cb)(int tap_id, const unsigned char *bgra, unsigned width, unsigned height);

typedef struct jf_vtap {
    pjmedia_port base;
    pj_pool_t *pool;
    jf_video_cb cb;
    int id;
    unsigned width, height;
} jf_vtap;

static pj_status_t jf_vtap_put_frame(pjmedia_port *port, pjmedia_frame *frame)
{
    jf_vtap *tap = (jf_vtap *)port;
    if (tap->cb && frame->type == PJMEDIA_FRAME_TYPE_VIDEO && frame->size >= (pj_size_t)tap->width * tap->height * 4)
        tap->cb(tap->id, (const unsigned char *)frame->buf, tap->width, tap->height);
    return PJ_SUCCESS;
}

static pj_status_t jf_vtap_get_frame(pjmedia_port *port, pjmedia_frame *frame)
{
    PJ_UNUSED_ARG(port);
    frame->type = PJMEDIA_FRAME_TYPE_NONE;
    frame->size = 0;
    return PJ_SUCCESS;
}

/* The bridge removes ports asynchronously, so the tap owns its pool and
 * releases it once the last reference is gone. */
static pj_status_t jf_vtap_on_destroy(pjmedia_port *port)
{
    jf_vtap *tap = (jf_vtap *)port;
    tap->cb = NULL;
    if (tap->pool)
        pj_pool_safe_release(&tap->pool);
    return PJ_SUCCESS;
}

/* Adds a width x height BGRA video tap to the video bridge and connects
 * `source` (a video bridge slot) to it. Returns the tap's slot, or -1.
 * Remove it with pjsua_vid_conf_remove_port() then pjmedia_port_destroy(). */
int jf_video_tap_add(int tap_id, jf_video_cb cb, int source, unsigned width, unsigned height, pjmedia_port **p_port)
{
    pj_pool_t *pool;
    jf_vtap *tap;
    pjmedia_format fmt;
    pj_str_t name = { "jf-vtap", 7 };
    pjsua_conf_port_id slot = PJSUA_INVALID_ID;

    pool = pjsua_pool_create("jf-vtap", 512, 512);
    if (!pool)
        return -1;
    tap = PJ_POOL_ZALLOC_T(pool, jf_vtap);
    pjmedia_format_init_video(&fmt, PJMEDIA_FORMAT_BGRA, width, height, 30, 1);
    pjmedia_port_info_init2(&tap->base.info, &name, PJMEDIA_SIG_CLASS_PORT_VID('J', 'V'), PJMEDIA_DIR_DECODING, &fmt);
    tap->base.put_frame = &jf_vtap_put_frame;
    tap->base.get_frame = &jf_vtap_get_frame;
    tap->base.on_destroy = &jf_vtap_on_destroy;
    tap->pool = pool;
    tap->cb = cb;
    tap->id = tap_id;
    tap->width = width;
    tap->height = height;
    if (pjsua_vid_conf_add_port(pool, &tap->base, NULL, &slot) != PJ_SUCCESS) {
        pj_pool_release(pool);
        return -1;
    }
    if (pjsua_vid_conf_connect(source, slot, NULL) != PJ_SUCCESS) {
        pjsua_vid_conf_remove_port(slot);
        pjmedia_port_destroy(&tap->base);
        return -1;
    }
    *p_port = &tap->base;
    return slot;
}
