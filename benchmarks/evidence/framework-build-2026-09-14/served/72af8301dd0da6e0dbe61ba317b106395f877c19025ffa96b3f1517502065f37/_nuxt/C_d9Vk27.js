import { a as createElementBlock, c as openBlock, d as isRef, f as ref, i as createBaseVNode, l as withCtx, m as toDisplayString, o as createTextVNode, p as unref, r as vModelText, s as createVNode, t as NuxtLink, u as withDirectives } from "#entry";
//#region pages/index.vue
var _hoisted_1 = { "data-framework": "nuxt" };
var _sfc_main = {
	__name: "index",
	setup(__props) {
		const count = ref(0);
		const query = ref("");
		function add() {
			count.value++;
			localStorage.setItem("nuxt-count", String(count.value));
		}
		return (_ctx, _cache) => {
			const _component_NuxtLink = NuxtLink;
			return openBlock(), createElementBlock("main", _hoisted_1, [
				_cache[3] || (_cache[3] = createBaseVNode("h1", null, "Nuxt ready", -1)),
				createBaseVNode("label", null, [_cache[1] || (_cache[1] = createTextVNode("Search", -1)), withDirectives(createBaseVNode("input", {
					"onUpdate:modelValue": _cache[0] || (_cache[0] = ($event) => isRef(query) ? query.value = $event : null),
					"aria-label": "Search"
				}, null, 512), [[vModelText, unref(query)]])]),
				createBaseVNode("button", { onClick: add }, "Add item"),
				createBaseVNode("output", null, "Items: " + toDisplayString(unref(count)), 1),
				createVNode(_component_NuxtLink, { to: "/details" }, {
					default: withCtx(() => [..._cache[2] || (_cache[2] = [createTextVNode("Route", -1)])]),
					_: 1
				})
			]);
		};
	}
};
//#endregion
export { _sfc_main as default };
