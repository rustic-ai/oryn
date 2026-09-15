import { a as createElementBlock, c as openBlock, i as createBaseVNode, l as withCtx, o as createTextVNode, s as createVNode, t as NuxtLink } from "#entry";
import { t as _plugin_vue_export_helper_default } from "./BOaGB7Aw.js";
//#region pages/details.vue
var _sfc_main = {};
function _sfc_render(_ctx, _cache) {
	const _component_NuxtLink = NuxtLink;
	return openBlock(), createElementBlock("main", null, [_cache[1] || (_cache[1] = createBaseVNode("h1", null, "Nuxt details", -1)), createVNode(_component_NuxtLink, { to: "/" }, {
		default: withCtx(() => [..._cache[0] || (_cache[0] = [createTextVNode("Back", -1)])]),
		_: 1
	})]);
}
var details_default = /*#__PURE__*/ _plugin_vue_export_helper_default(_sfc_main, [["render", _sfc_render]]);
//#endregion
export { details_default as default };
