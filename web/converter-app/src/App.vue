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
  <nav class="flex w-full justify-evenly">
      <a href="#/">Registration</a> |
      <a href="#/search">Search</a>
  </nav>
  <component :is="currentView" />
</div>

</template>

<style>
button {
  @apply rounded bg-indigo-500 hover:bg-indigo-600 text-white p-2 w-24 h-16;
  @apply disabled:bg-gray-500;
  @apply disabled:cursor-progress;
}

</style>
