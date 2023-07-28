<script setup lang="ts">
import { ref, computed } from 'vue'

import Registration from './components/Registration.vue'
import Search from './components/Search.vue'
import NotFound from './components/NotFound.vue'

const routes = {
  '/': Registration,
  '/search': Search, 
}

const currentPath = ref(window.location.hash)

window.addEventListener('hashchange', () => {
  currentPath.value = window.location.hash
})

const currentView = computed(() => {
  return routes[currentPath.value.slice(1) || '/'] || NotFound
})
</script>

<template>

<div>
  <nav class="flex w-full justify-evenly border-indigo-500 border-solid border-b-2 mb-2">
      <a href="#/" :class="{'current': currentPath === '#/'}">Registration</a>
      <a href="#/search" :class="{'current': currentPath === '#/search'}">Search</a>
  </nav>
  <KeepAlive>
    <component :is="currentView" />
  </KeepAlive>
</div>

</template>

<style>
a {
  @apply rounded-t bg-indigo-500 hover:bg-indigo-600 text-white text-center p-2 w-36;
}

a.current {
  @apply bg-indigo-800 pointer-events-none;
}

</style>
