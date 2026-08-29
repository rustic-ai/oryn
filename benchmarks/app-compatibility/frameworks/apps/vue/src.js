import {createSSRApp,h,ref} from 'vue';
const App={setup(){const count=ref(0),query=ref('');return()=>h('main',{'data-framework':'vue'},[h('h1','Vue ready'),h('label',['Search',h('input',{'aria-label':'Search',value:query.value,onInput:event=>query.value=event.target.value})]),h('button',{onClick:()=>{count.value++;localStorage.setItem('vue-count',String(count.value));}},'Add item'),h('output','Items: '+count.value),h('a',{href:'?route=details'},'Route')]);}};
createSSRApp(App).mount('#app');
