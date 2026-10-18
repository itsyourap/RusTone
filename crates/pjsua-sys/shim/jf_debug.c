/*
 * Debug taps for RusTone's debug window: a SIP message tap, flattened stream
 * statistics and audio taps on the conference bridge.
 */
#include <pjsua-lib/pjsua.h>

/* ---- SIP message tap: a module just above the transport layer that sees
 * every message sent and received as raw text (pjsua's own message logger
 * uses the same priority). */

typedef void (*jf_msg_cb)(int is_rx, const char *buf, size_t len,
                          const char *remote, int port, const char *transport);

static jf_msg_cb jf_msg_callback;

static pj_bool_t jf_msg_on_rx(pjsip_rx_data *rdata)
{
    if (jf_msg_callback && rdata->msg_info.msg_buf) {
        jf_msg_callback(1, rdata->msg_info.msg_buf, (size_t)rdata->msg_info.len,
                        rdata->pkt_info.src_name, rdata->pkt_info.src_port,
                        rdata->tp_info.transport ?
                            rdata->tp_info.transport->type_name : "");
    }
    return PJ_FALSE;
}

static pj_status_t jf_msg_on_tx(pjsip_tx_data *tdata)
{
    if (jf_msg_callback && tdata->buf.start) {
        jf_msg_callback(0, tdata->buf.start,
                        (size_t)(tdata->buf.cur - tdata->buf.start),
                        tdata->tp_info.dst_name, tdata->tp_info.dst_port,
                        tdata->tp_info.transport ?
                            tdata->tp_info.transport->type_name : "");
    }
    return PJ_SUCCESS;
}

static pjsip_module jf_msg_module = {
    NULL, NULL,                                 /* prev, next          */
    { "mod-rustone-msg-tap", 17 },                /* name                */
    -1,                                         /* id                  */
    PJSIP_MOD_PRIORITY_TRANSPORT_LAYER - 1,     /* priority            */
    NULL, NULL, NULL, NULL,                     /* load/start/stop/unl */
    &jf_msg_on_rx,                              /* on_rx_request       */
    &jf_msg_on_rx,                              /* on_rx_response      */
    &jf_msg_on_tx,                              /* on_tx_request       */
    &jf_msg_on_tx,                              /* on_tx_response      */
    NULL,                                       /* on_tsx_state        */
};

/* Registers the tap on pjsua's endpoint (after pjsua_init). */
pj_status_t jf_msg_tap_install(jf_msg_cb cb)
{
    jf_msg_callback = cb;
    jf_msg_module.id = -1;
    return pjsip_endpt_register_module(pjsua_get_pjsip_endpt(), &jf_msg_module);
}

/* Unregisters the tap (before pjsua_destroy). */
void jf_msg_tap_remove(void)
{
    if (jf_msg_module.id != -1)
        pjsip_endpt_unregister_module(pjsua_get_pjsip_endpt(), &jf_msg_module);
    jf_msg_callback = NULL;
}

/* ---- Stream statistics, flattened for Rust. */

typedef struct jf_stream_snapshot {
    char codec[32];
    unsigned clock_rate;
    unsigned channels;
    char remote[64];
    unsigned duration_ms;
    unsigned tx_pkt, tx_bytes, rx_pkt, rx_bytes;
    unsigned rx_loss, rx_discard, rx_dup, rx_reorder;
    unsigned rx_jitter_last_us, rx_jitter_mean_us, rx_jitter_max_us;
    /* As reported back by the remote in RTCP. */
    unsigned tx_loss, tx_jitter_last_us;
    unsigned rtt_last_us, rtt_mean_us;
    unsigned jb_frame_bytes, jb_size, jb_prefetch, jb_avg_delay_ms;
    unsigned jb_lost, jb_discard, jb_empty;
} jf_stream_snapshot;

size_t jf_sizeof_jf_stream_snapshot(void) { return sizeof(jf_stream_snapshot); }

/* Fills `out` for the call's first audio stream. */
pj_status_t jf_call_stream_snapshot(pjsua_call_id call_id, jf_stream_snapshot *out)
{
    pjsua_call_info ci;
    pjsua_stream_info si;
    pjsua_stream_stat st;
    unsigned i, med = (unsigned)-1;
    pj_status_t status;
    size_t n;
    pj_time_val now;

    pj_bzero(out, sizeof(*out));
    status = pjsua_call_get_info(call_id, &ci);
    if (status != PJ_SUCCESS)
        return status;
    for (i = 0; i < ci.media_cnt; ++i) {
        if (ci.media[i].type == PJMEDIA_TYPE_AUDIO) {
            med = ci.media[i].index;
            break;
        }
    }
    if (med == (unsigned)-1)
        return PJ_ENOTFOUND;

    status = pjsua_call_get_stream_info(call_id, med, &si);
    if (status != PJ_SUCCESS)
        return status;
    n = (size_t)si.info.aud.fmt.encoding_name.slen;
    if (n > sizeof(out->codec) - 1)
        n = sizeof(out->codec) - 1;
    pj_memcpy(out->codec, si.info.aud.fmt.encoding_name.ptr, n);
    out->codec[n] = '\0';
    out->clock_rate = si.info.aud.fmt.clock_rate;
    out->channels = si.info.aud.fmt.channel_cnt;
    if (pj_sockaddr_has_addr(&si.info.aud.rem_addr))
        pj_sockaddr_print(&si.info.aud.rem_addr, out->remote, sizeof(out->remote), 3);

    status = pjsua_call_get_stream_stat(call_id, med, &st);
    if (status != PJ_SUCCESS)
        return status;
    /* pjmedia stamps the stream start with wall-clock time. */
    pj_gettimeofday(&now);
    PJ_TIME_VAL_SUB(now, st.rtcp.start);
    out->duration_ms = (unsigned)PJ_TIME_VAL_MSEC(now);
    out->tx_pkt = st.rtcp.tx.pkt;
    out->tx_bytes = st.rtcp.tx.bytes;
    out->rx_pkt = st.rtcp.rx.pkt;
    out->rx_bytes = st.rtcp.rx.bytes;
    out->rx_loss = st.rtcp.rx.loss;
    out->rx_discard = st.rtcp.rx.discard;
    out->rx_dup = st.rtcp.rx.dup;
    out->rx_reorder = st.rtcp.rx.reorder;
    out->rx_jitter_last_us = st.rtcp.rx.jitter.last;
    out->rx_jitter_mean_us = (unsigned)st.rtcp.rx.jitter.mean;
    out->rx_jitter_max_us = st.rtcp.rx.jitter.max;
    out->tx_loss = st.rtcp.tx.loss;
    out->tx_jitter_last_us = st.rtcp.tx.jitter.last;
    out->rtt_last_us = st.rtcp.rtt.last;
    out->rtt_mean_us = (unsigned)st.rtcp.rtt.mean;
    out->jb_frame_bytes = st.jbuf.frame_size;
    out->jb_size = st.jbuf.size;
    out->jb_prefetch = st.jbuf.prefetch;
    out->jb_avg_delay_ms = st.jbuf.avg_delay;
    out->jb_lost = st.jbuf.lost;
    out->jb_discard = st.jbuf.discard;
    out->jb_empty = st.jbuf.empty;
    return PJ_SUCCESS;
}

/* ---- Audio tap: a conference port that hands every frame it receives to
 * a callback (level meters, waveforms, spectra). */

typedef void (*jf_tap_cb)(int tap_id, const short *samples, size_t count,
                          unsigned clock_rate);

typedef struct jf_tap {
    pjmedia_port base;
    pj_pool_t *pool;
    jf_tap_cb cb;
    int id;
} jf_tap;

static pj_status_t jf_tap_put_frame(pjmedia_port *port, pjmedia_frame *frame)
{
    jf_tap *tap = (jf_tap *)port;
    if (tap->cb && frame->type == PJMEDIA_FRAME_TYPE_AUDIO && frame->size) {
        tap->cb(tap->id, (const short *)frame->buf, frame->size / 2,
                PJMEDIA_PIA_SRATE(&port->info));
    }
    return PJ_SUCCESS;
}

/* The bridge removes ports asynchronously, so the tap owns its pool and
 * releases it here, once the last reference is gone (pjproject PR #3928). */
static pj_status_t jf_tap_on_destroy(pjmedia_port *port)
{
    jf_tap *tap = (jf_tap *)port;
    tap->cb = NULL;
    if (tap->pool)
        pj_pool_safe_release(&tap->pool);
    return PJ_SUCCESS;
}

static pj_status_t jf_tap_get_frame(pjmedia_port *port, pjmedia_frame *frame)
{
    PJ_UNUSED_ARG(port);
    frame->type = PJMEDIA_FRAME_TYPE_NONE;
    frame->size = 0;
    return PJ_SUCCESS;
}

/* Creates a tap in the conference bridge's format and adds it to the
 * bridge. Returns its slot, or -1. Remove it with pjsua_conf_remove_port()
 * then pjmedia_port_destroy(*p_port); its pool is released in on_destroy. */
int jf_tap_add(int tap_id, jf_tap_cb cb, pjmedia_port **p_port)
{
    pjsua_conf_port_info master;
    pj_pool_t *pool;
    jf_tap *tap;
    pj_str_t name = { "jf-tap", 6 };
    int slot = PJSUA_INVALID_ID;

    if (pjsua_conf_get_port_info(0, &master) != PJ_SUCCESS)
        return -1;
    pool = pjsua_pool_create("jf-tap", 512, 512);
    if (!pool)
        return -1;
    tap = PJ_POOL_ZALLOC_T(pool, jf_tap);
    pjmedia_port_info_init(&tap->base.info, &name,
                           PJMEDIA_SIG_CLASS_PORT_AUD('J', 'T'),
                           master.clock_rate, master.channel_count, 16,
                           master.samples_per_frame);
    tap->base.put_frame = &jf_tap_put_frame;
    tap->base.get_frame = &jf_tap_get_frame;
    tap->base.on_destroy = &jf_tap_on_destroy;
    tap->pool = pool;
    tap->cb = cb;
    tap->id = tap_id;
    if (pjsua_conf_add_port(pool, &tap->base, &slot) != PJ_SUCCESS) {
        pj_pool_release(pool);
        return -1;
    }
    *p_port = &tap->base;
    return slot;
}
